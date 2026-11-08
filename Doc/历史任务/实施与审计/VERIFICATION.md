> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 性能预算与纵向验收

### 最新统一复验检查点（2026-10-12）

**2026-10-13 修改后的任务不能退回普通回答；真实编程主线通过。**
Core 使用现有非临时 mutation_count 守卫：显式 Answered 提案拒绝，未评估且无提案的
正文收尾为 Incomplete(requirements.unassessed)，不自动重试或制造工具调用。
requirements 整组 11 passed，`/tmp/nola-action-result-test.log`，覆盖零修改普通回答、
修改后显式回答拒绝/回滚、无提案的未完成及无新增 effect。
新 Test 构建真实 CSV 任务 `/tmp/nola-mac-coding.0Lmphe`：零工具错误，正式
action_completed 审计通过，独立 Unicode/负数/十进制输入执行通过；取消正式封存
通过（约 186ms 单样本）。完整 runner exit 9 唯一失败为
large_response_delivered_incrementally：2214 字正文、1 次展示变更。
日志 `/tmp/nola-coding-action-result.log`。不将 UI 检查失败改写为完整 runner 成功；
按用户主线优先级暂不扩大展示修复。此守卫不声称覆盖所有远程副作用。

**2026-10-13 完整编程任务重复真实验收：取消稳定，完成分类仍有反例。**
同一 CSV 编程任务、显式 Responses low、当前 Test 构建：
XoLwkh 产物独立验证通过，主 Run action_completed、取消通过，但模型把只读输入
sample.csv 声明为 edit，触发一次真实 postcondition failure，整体 exit 9。
工具 schema 已澄清 output_artifacts 不包含输入文件/stdout，未放宽实际修改观测。
新构建 6vrSmE 独立程序验证仍通过、取消通过，但出现两次缺 validation_targets 和
一次不支持的 shell 语法；主 Run 最终为 answered 而非 action_completed，完整 runner
exit 9。日志 `/tmp/nola-coding-output-contract.log`。这不是整体成功。
反例定位：requirements.rs::decide_requirement_result 在无显式提案且债务清除后可
退回 Answered；当前引导不能保证每次实际任务均提供 requirement assessments。
下一主线需要区分“查询辅助工具的回答”与“外部动作任务的未评估交付”，不能粗暴
按调用过工具就强制 ActionCompleted，也不能把模型没提交评估当成普通回答成功。

**2026-10-13 真实模型期间取消收尾通过。**
中断单次提交/重排/边界源重建回归 1 passed，
`/tmp/nola-cancel-reentry-test.log`；此前扩展 boundary 套件的旧 fixture 失败仍未消除。
最新 `/tmp/nola-mac-executable.cvP5KL`，日志 `/tmp/nola-real-cancel-shared.log`，
完整 runner exit 0，独立脚本直接执行通过；SQLite 两个 Run 分别 sealed/action_completed
和 sealed/cancelled。取消在持久准入后发起，正式终态查询通过，耗时约 175ms；
这是单样本，不是 p95 或底层所有进程退出的证据。
后续修复包含：Host 错误后先恢复已提交执行状态；已 Cancelling/terminal 时不重复
提交 Interrupt；边界源队列/中断发出记录归属 Turn 驱动，在多次 effect 排空之间共享，
只在 begin_turn 重置。此前每次 boundary_submission_source() 新建状态导致反复中断，
Core 报 already cancelling，留下 pending cancel effect。
失败样本 Dua9D5/9wOwXh/s5R4Ej/8nY4YT 保留；诊断入口不再吞掉缺 trace 错误的
有界 detail，仍按协议错误拒绝，不赋予其业务终态权威。Mac 取消探针排空完整流后
读 canonicalRunTerminal，不拿取消事件替代 seal。

**2026-10-13 取消边界整改进行中，真实终态仍不通过。**
CliBoundarySubmissionSource 在接纳 Host 终止响应前，将执行控制中断排队为
AgentSubmission::Interrupt；每执行只发一次，重排保留同一 ID。新增回归覆盖单次提交、
重排及 ACK 后不重复。`nola_cargo test -p nola_cli --lib boundary`：38 passed、
1 failed（旧 terminal_transport_waits_until_housekeeping_boundary fixture 缺
no_progress_recovery），日志 `/tmp/nola-cancel-boundary.log`；不得称整套通过。
首次编译误用 Cancel variant，改为正式 Interrupt 后编译通过。
真实 Pu9B0J 已记录 Core cancellation fact 而非 runtime_failure，但未封存。
移除验收器收到 Core terminal 就 break 的提前退出后重跑
`/tmp/nola-mac-executable.KVnK4f` 仍 exit 9：主任务交付成功，取消 Run 仍 running，
canonical terminal 缺失。日志 `/tmp/nola-real-cancel-drained.log`。
因此取消准入问题已有改进，但 Host cancel effect 排空/最终 seal 链路尚未完成；
不能以 Core cancellation fact 或流结束声明 Cancelled 已落库。

**2026-10-13 思考配置传递修复；真实取消竞态仍未解决。**
验收主任务、续聊、取消请求构造遗漏 provider.reasoning/parameters，现传递完整配置与
balanced 用户档位，由 Application 统一解析（测试声明仅支持 low，实际选择 low）。
新构建真实 `/tmp/nola-mac-executable.NYZSVV`：四次主模型请求 probe 均记录
reasoningControl={effort:low}，主任务 action_completed、独立脚本执行通过；
不据单样本宣称性能提升。取消 Run 正式结果却为 Failed，其 Core runtime_failure 为
“Host execution stopped without an admitted cancellation: ... The model request was cancelled.”
因此并非单纯缺少 UI run.cancelled 事件，而是取消准入与 Host 停止的竞争。
取消验收现改读生产 canonicalRunTerminal 并严格要求 Cancelled，不依据 Trace 判定。
再次真实构建复验仍 exit 9（runtime=1，persistence=0；脚本直接执行通过），
日志 `/tmp/nola-real-low-effort-canonical.log`。下一主线修复必须先追踪取消准入/网络
中止顺序，不放宽 Failed→Cancelled 判定或删除探针。

**2026-10-13 思考控制真实传递核查，尚未通过。**
Mac 验收入口增加 `NOLA_MAC_REAL_REASONING_JSON`，复用生产
MacReasoningConfiguration 解码、协议/输出预算/能力校验，禁止覆写启用 false 能力。
MacRuntimeAcceptanceModelFixtureTests 的 Test 配置 XCTest exit 0，日志
`/tmp/nola-reasoning-fixture-tests.log`。不改用户配置或生产凭据。
声明 Responses effort=low 后真实脚本任务 `/tmp/nola-mac-executable.mMmrjY`：
任务文件直接运行通过，canonical/persistence 审计通过、正文完整、零工具错误；
但四次主请求的 probe reasoningControl 仍为 null，不能当作低思考档验收。
说明配置传入与实际 selection 仍有链路缺口，必须继续追踪，不能仅凭配置字段非空通过。
独立取消探针 missing run.cancelled，run_success=false，完整 runner 未通过；
日志 `/tmp/nola-real-low-effort.log`，保留失败而非删去取消检查。

**2026-10-13 真实可执行脚本交付：权限位证据与终态答复。**
从 XXr1Sm 的 chmod 误报定位到 Host 文件变化指纹仅包含内容/大小；普通文件观测
现包含打开句柄的权限位（不靠命令词匹配），内容未变但 mode 改变可正确形成 edit。
权限变化/不变/恢复原 mode 回归 1 passed，`/tmp/nola-artifact-mode-test.log`。
`scripts/test_real_mac_executable_task.sh` 首轮 IJh0JD：真实 write_file、chmod、直接
执行均成功，但模型 propose_result 未附 answer，正式任务已收尾而正文为空。
现在所有模型终态提案要求非空顶层 answer，缺失整批回滚可纠正，工具说明明确不会有
后继答复；requirements 整组 10 passed（含空答案回滚和补全后成功），
`/tmp/nola-result-answer-test.log`。系统取消/预算终态不受此模型调用条件影响。
新 Test 构建真实重跑 `/tmp/nola-mac-executable.ZiqAyv`，Mac 通用 runner passed：
零工具错误、用户收到一句说明、action_completed 及持久结果审计通过。
外围脚本最初 exit 1，因为模型在同次 chmod 调用追加只读 `&& ls -l hello.sh`，
精确命令断言拒绝；验收只增加这一已观测的等价形式，未放宽生产边界。
对同份真实产物离线重验通过（直接执行 stdout 精确匹配）。未再调用上游，
故不把外围最初 exit 1 改写为整条命令 exit 0。日志 `/tmp/nola-real-executable-final.log`。

**2026-10-13 真实编程任务发现并修复关闭文本误复用。**
新增 `scripts/test_real_mac_coding_task.sh`：真实配置模型实现 CSV 十进制分类汇总，
自行创建样例并运行；任务结束后由验收器创建另一份 Unicode/负数/小数输入，独立运行交付程序。
首次 `/tmp/nola-mac-coding.XXr1Sm`：实际程序通过，但 runtime 因
`canonical Text planning failed: Closed` 失败，整体 exit 9。
Core 完成项投影曾把前一工具轮次已关闭的空 Text 当作候选文本复用；现候选合并仅允许
尚未完成、同 Turn 且前缀一致的 Text，不放宽 Store 的关闭不可写契约。
新增回归验证旧关闭块保持不变、后续答案创建独立块；projection 整组 9 passed，
`/tmp/nola-closed-text-regression.log`。首次测试 fixture 缺 session_id，补齐后通过。
重新构建 Test 后真实重跑 `/tmp/nola-mac-coding.fZy6qS`：runtime=0，正式
Run/Result/Seal 审计为 action_completed，独立程序验收通过。总 runner 仍 exit 9，
唯一未通过审计为 large_response_delivered_incrementally（1770 字正文、1 次展示变更）；
不能声明整套 UI 验收通过。当前按用户优先级聚焦实际任务执行，展示项单独保留。

**2026-10-13 真实写入成功、验收失败，按未完成封存。**
`bash scripts/test_real_mac_incomplete_validation.sh`，Responses、现有 Test 构建，
`/tmp/nola-mac-incomplete.44u5z1`，runner exit 0。实际事件为一次 write_file、
一次 bash 验证（is_error=true），随后 requirements_update 将要求评为 unsatisfied，
提出 incomplete；正式结果审计接受 incomplete，独立文件检查仍为 42。
没有重写文件或重试验证。此例只证明明确失败验收，不代表未知副作用或断线恢复。

**2026-10-13 等待回答后的无内存快照恢复通过。**
`/tmp/nola-mac-real-memory.WCx0Rr/result.xcresult`：1 passed、0 failed、0 skipped，
runner exit 0。在 TdO9UL 用例基础上不再复用原 Runtime，而新建 Runtime，通过
prepareColdResume 从持久调用、原模型凭据引用及权限重建，再完成原 Run。
这不是整个 App 进程 kill 的验收，不能扩大为所有崩溃交错已覆盖。
新增 Application+SQLite 回归 prepared_model_attempt_survives_rebind_without_sampling_mutation：
关闭重开库后重绑未开始采样的模型 effect，明确核对新 request ID、原 effect/Attempt ID、
attempt_started=false 及实际 claim 成功。整组 10 passed，
`/tmp/nola-prepared-rebind-regression.log`；不再仅靠真实模型样本覆盖这条恢复缺陷。

**2026-10-13 真实模型提问、持久等待与同 Run 恢复通过。**
`/tmp/nola-mac-real-memory.TdO9UL/result.xcresult`：1 passed / 0 failed / 0 skipped，
runner exit 0。测试重开 Application 查询问题，核验等待阶段无 Core terminal、
无 canonical seal；提交提问后才生成的随机文本答案，以原权限及隔离模型凭据引用
恢复原 Run，最后读取正式答案并核验 Answered。SQLite 只有 1 个 sealed/answered
Run、1 次 turn_started、1 次 turn_completed、2 次 wait 变更，不创建替代 Run。
复现命令 `bash scripts/test_real_mac_user_input.sh`。
修复前 xjNten 实际复现 `outbox delivery differs from its canonical Attempt`：回答
生成的 Model effect 已有 prepared Attempt，但恢复重绑把 attempt_id 清空。Application
现从同 owner 的正式活动 Attempt 恢复小 binding；9 项 Application 回归通过。
前置 fixture 失败 2LyBch/FkA29Q/HEzAY7 分别是未捕获类型化等待、权限不一致、
临时直连无可恢复凭据；保留生产校验，改测试配置而非绕过。此样本不证明进程崩溃、
多端同时答复、ACK 丢失及所有 UI 窗口均通过。

**2026-10-13 Mac 真实取消及同会话恢复通过。**
`/tmp/nola-mac-real-memory.Ig2vdv/result.xcresult`：1 passed / 0 failed / 0 skipped，
runner exit 0；隔离测试批准本次权限，在 bash toolStarted 后发出取消。
测试通过生产 Application canonicalRunTerminal 分别核验旧 Run Cancelled 和同会话
新 Run Answered，SQLite 也为 sealed/cancelled、sealed/answered。取消等待的 10 秒
检查从取消发起时计时，不再把冷模型预填充算入；这是测试保护界限，不是实测 p95 SLA。
终态断言只使用 Core terminal，不将 accompanying Trace 作为第二次完成。
失败对照 iHDDTW 保留：旧测试未处理权限卡，45 秒超时前根本没开始工具，不能
用它评价运行中工具取消性能。此验收基于工具开始事件，不宣称已核验底层进程树
全部退出或副作用重放；这些需要独立进程/执行证据，不能由取消状态推导。

**2026-10-13 拒绝授权真实路径修正并通过。**
原样本 `/tmp/nola-mac-real-e2e.wTjWJ8/` 在用户拒绝后仍落 answered，旧“成功回答”
门禁误放行。Core 现根据已记录 user_denied_permission 写入阻断事实，收尾为
blocked/permission.denied；取消、失败、未知副作用仍保留原优先级，拒绝不等于
仍有活动授权等待。无工具说明回答不能把任务提升为成功，也不要求模型再调控制工具。
新构建 `/tmp/nola-mac-denial-acceptance.aMyrOY/` 完整 runner exit 0：主任务
blocked/permission.denied，独立取消探针 cancelled，目标文件不存在。批准写入
wrapper 改为明确期待 action_completed；拒绝 wrapper 明确期待 blocked，不再共用
泛化成功结果断言。Core 要求回归 9 passed、审计反例 7 passed，日志
`/tmp/nola-denial-result-tests.log` 与 `/tmp/nola-denial-blocked-real.log`。

**2026-10-13 真实 Mac 授权写入完整 runner 通过。**
`/tmp/nola-mac-write-acceptance.YR8ltE/`：模型写文件、读回、登记要求评估，
主任务 action_completed，独立取消探针 cancelled；实际交付文件核对正确，
runtime-session-audit passed=true、无失败断言，wrapper exit 0。
审计不再以 trace_journal_runs 推断 Run 终态，而交叉核对 runs、run_results、
assistant_run_terminal_seals 与 run_events 的身份、结果、唯一决定和终态顺序。
时间范围由正式 Run 限定，Responses/control 提交答案不再误判为没有答案。
7 项缺失/冲突反例测试通过；此前 DnK1FC 离线复核也通过，但没有改写原失败回执。
边界：此 Python gate 是关系一致性检查，不重新实现 Rust 的 seal/manifest 密码学
校验；旧 Trace 工具诊断仍可缺省，完整 Host inbox/outbox 与权限消耗的逐条对账
不由该 gate 证明。不得据此把全端、所有恢复与权限契约标成完成。

真实写入任务的要求收尾复验 `/tmp/nola-mac-write-acceptance.DnK1FC/`：新构建中
模型执行 write_file → read_file(validate) → requirements_update，实际文件核对正确，
业务 RunResult 已为 action_completed，绑定 assessment:32，而非前两轮 answered。
运行投影 success=true；独立取消探针另落 cancelled，不与主任务混淆。
此次变更仅补足模型控制上下文的行动收尾规则及已有证据 source 关联，没有自动
将工具成功提升为任务成功，也未复制完整回执进 checkpoint。Core 要求测试 8 passed，
日志 `/tmp/nola-requirement-guidance-tests.log`。整体验收仍 exit 9：旧 Trace 审计
报 agent_run_present、agent_runs_terminal_success、trace_present、
single_terminal_event_per_latest_run、single_terminal_provider_answer；尚不可登记全链通过。

真实 Mac 授权写入样本 `/tmp/nola-mac-write-acceptance.wPza34/`：一张授权卡、
write_file 与 read_file(validate) 各一次，工具无错误，实际文件内容 APPROVED_PROBE。
但整体 runner exit 9，不能登记通过：canonical turn terminal 与 run.completed Trace
都被解码成 finished；审计脚本仍要求旧 trace_journal_runs 记录；实际 RunResult 为
answered 而非 action_completed。单有正确文件和回答不能证明要求/证据交付契约闭环。
已将 run.completed/failed/cancelled 诊断 Trace 解码为 status，保留 Trace 细节，
仅 canonical Core terminal 发出生命周期事件；Test 配置的两项定向回归 exit 0，
`/tmp/nola-terminal-projection-regression.log`。任务要求准入与审计来源仍需整改。
新构建真实复验 `/tmp/nola-mac-write-acceptance.xTLkBP/`：finished 恢复为 1，
runtime success=true，文件内容直接核对正确；整体仍 exit 9，失败留在旧 Trace
审计和 Responses 答案识别，未通过放宽断言掩盖。验收 wrapper 现独立核对实际文件，
并保留 runtime 原失败状态，不以文件成功覆盖其他失败。

**修正测试超时后，真实 Responses 截断续写及全部门禁通过。**
`/tmp/nola-mac-real-memory.abMTpK/result.xcresult`：新构建 Test 配置，
1 passed / 0 failed / 0 skipped，runner exit 0。上游一次摘要输出额度为 192，
真实 max_output_tokens 截断；646 字节片段经一次续写被完整提交，原文工具召回通过。
15 次模型请求均有 usage、无 transport error，最大实际输入 29666；加请求输出
16384 与安全余量 3200，小于 64000 硬窗口。6 个 Run 全部 sealed/answered。
总输入 362599、cache read 207642（约 57.27%）；摘要首请求 2865 ms、续写
7381 ms，仅为两个物理调用时长，不是压缩事务总耗时。此样本证明真实截断恢复，
不代表自然饱和窗口、所有协议或 UI 绘制已验收。复现入口：
`bash scripts/test_real_mac_compaction_continuation.sh`，要求配置端点支持 Responses。

**Responses 真实截断续写已触发，但首次整体验收被计量门禁正确拒绝。**
`/tmp/nola-mac-real-memory.jynhPT/result.xcresult` 的 XCTest 1 passed：
测试代理只把一次压缩请求的上游输出额度设为 192，不合成 SSE。上游真实返回
max_output_tokens 和 683 字节片段；Store 关联同一 effect 的 continuation_count=1、
partial_summary_bytes=683 及随后 compaction_completed，后续原文召回成功。
然而首个正常请求出现 responses_timeout、无 usage，重试才成功，因此不能宣称
全部物理请求预算/计量已验证。qP7czm 同配置复现，保留两次失败证据。
源码根因：MacLocalAgentRuntime 对所有 XCTest 强制 5 秒流超时，包括真实 LLM。
已限定短超时只用于非真实模型测试，真实验收沿用正常运行配置；新构建复验通过，见 abMTpK。

**真实 Mac 合法预算内压缩、续聊及原文召回通过。**
证据 `/tmp/nola-mac-real-memory.nNR5lx/result.xcresult`：1 passed / 0 failed / 0 skipped，
测试阶段 45.745 秒（不是单次压缩耗时）。固定 W=64000 / O=16384 / P=44416 /
T=30000 / target=18000，未改变 raxVE2 失败复现的配置。
Provider 共 9 次生成请求，全部 HTTP 200、输出额度均为 16384；最大实际输入
28302，小于 P。总输入 210845、缓存读取 125563，按 token 加权约 59.55%。
热分叉摘要输入 28302、缓存读取 27682（约 97.81%）、输出 464 token；随后正常
输入降至 21200。DB 6 个 Run 全部 sealed/answered，compaction_generation=1。
测试明确验证压缩事件、原始 transcript 保留、conversation_recall 找回随机原文事实。
这证明本次可容纳窗口的真实主链；不证明 16k 窗口可运行同样静态配置、自然满窗、
真实输出截断续写、UI 绘制或手动压缩/服务端超窗恢复所有入口已统一。

压缩验收脚本现默认使用上述 64k 硬窗口、30k 触发阈值，并强制检查实际
provider 输入 usage + 请求输出额度 + 安全余量不超过窗口；缺少 usage 不算通过。
`provider_budget_acceptance` 的 9 项测试通过；对 nNR5lx 原始 trace 重放检查，
9 次请求全部满足预算。这是已有真实请求的证据复核，不是新增一次模型运行。
小窗口预算受阻仍由独立的零调用用例覆盖，不与成功压缩混为一谈。

合法硬预算内的压缩实测暴露软目标错误：
`/tmp/nola-mac-real-memory.raxVE2/result.xcresult` 第 4 个续聊回合失败。
固定 W=64000、O=16384、P=44416、T=30000、目标=18000；前 4 次生成输入
16668/20335/24007/27667 token，均未超 P。热分叉摘要请求输入 28287、缓存读取
27671，但保存的 compact effect 明确要求 `Target at most 1 tokens`，最终报
`Compaction summary still exceeds its admitted budget; original context retained`。
根因是扣除非历史成本后的软目标为零，被 summary_token_budget 的 max(1) 变成
不可能满足结构要求的输出合同。已改为零软目标使用硬剩余空间，再由原文减量
约束限制预算；不是取消硬预算。`compaction_summary_budget` 3 passed，
`/tmp/nola-zero-soft-target-v2.log`。首轮新增测试漏算既有 archive savings 上限，
期望从 892 修正为 100，生产减量约束保留，原失败日志保留。
相同 W/T 的最新构建复验已通过，见上方 nNR5lx 记录。

最新 Mac 双路径复验：

- 零调用预算受阻：新增 `testRealConfiguredRoutePersistsContextBudgetBlockerWithoutModelCall`
  1 passed / 0 failed / 0 skipped，`/tmp/nola-mac-real-memory.x7HTuW/result.xcresult`。
  只读核验 DB 为 sealed/blocked/context.input_budget_exceeded，seal 1 条，outbox 0 条；
  provider trace 只有 `/v1/models`，无生成请求。这是实际路由与 Mac 持久化验收，
  不声称调用了 LLM。测试辅助代码只在 canonical 结果确认 Blocked 后认可无正文。
- 同一最新构建正常窗口真实 LLM：原 canonical answer 用例 1 passed / 0 failed /
  0 skipped，`/tmp/nola-mac-real-memory.x2gPiV/result.xcresult`；模型实际返回目标 token。
- 原 16k 压缩正向用例仍不成立：`/tmp/nola-mac-real-memory.UAMlsL/` 已能持久化
  受阻结果，但未进行压缩续聊。修复测试辅助代码不将这个事实改判为成功；仍须补足
  在合法完整请求预算内的真实压缩、原文检索及续聊验收。

真实 Mac 再次定位到状态化准入遗漏：`/tmp/nola-mac-real-memory.PFy9Hy/result.xcresult`，
错误变为 `Admitted Core has no canonical Run`；数据库中 Run 和 seal 均为零，证明
外层事务完整回滚，不能宣称 Mac 已修好。Runtime 准入后只查询 active_run，错误地
排除了合法的即时封存 Run。现已改为终态分支读取精确 Run invocation、校验 sealed /
canonical terminal 与配置摘要，不缓存无需执行的 Core；非终态路径保持原校验。
两项即时受阻测试已升级为经过 `ApplicationRuntime.execute`，成功持久化及故障回滚
均通过（2 passed，`/tmp/nola-immediate-runtime-terminal.log`）。最新 Runtime 改动
尚未重建 Mac 复验，原 9 项直接 Application 集成证据不能代替这一边界。

零 I/O 受阻终态已完成 Application→SQLite 纵向接线：
`effect_delivery_application_e2e` 9 passed，日志 `/tmp/nola-immediate-terminal-suite.log`。
新增测试验证超过预算的初次输入无 Host 请求，重开数据库后存在 canonical seal，
Run 为 sealed、结果为 blocked、原文逐字保留；另以 SQLite trigger 注入 seal
失败，验证 Run/消息/checkpoint/transition/结果全部回滚。
首轮发现“终态必须有非零 journal frontier”的额外假设，已仅为无 effect、无
内容的 Blocked 准入允许 genesis frontier；其他终态约束不放开。原失败日志
`/tmp/nola-immediate-terminal.log` 保留。**修改后的 Mac 真机链路尚未复验。**

最新 16k 真实复验失败，暴露零 I/O 终态准入缺口：
`/tmp/nola-mac-real-memory.8zhkR5/result.xcresult`，0 passed / 1 failed，
错误为 `application_rejected: staged Core transition has no external Host effect`。
本次复用上一轮已编译的最新 Test 产品，没有重启或修改生产会话。
Core 的预算受阻路径在初次准入可立即结束 Turn，但 Application assistant_turn
硬性要求非空 outbox；Store initial_run_events 也只允许 maintenance no-op 的
初始 turn_completed。因而先前 Core 单测通过不能证明受阻结果已可靠落库。
需要补齐正常 Run 的零 I/O 终态事务及发布；不得伪造 Host 请求绕过校验。

自动阈值压缩预算已预留完整请求成本：将 prompt/target 限额按已知 usage 校准
反算回历史估计单位，扣除非历史指令及工具成本，再规划保留历史与摘要。
无剩余空间时走明确上下文受阻结果，不发注定不能容纳的摘要请求。
`context_window::usage_snapshot_tests` 8 项通过，包含校准、恢复、余量及取整测试，
日志 `/tmp/nola-full-request-reserve-suite.log`。

最新构建真实 Mac 正常模型调用通过：
`testRealConfiguredModelProducesCanonicalAnswerThroughMacAgentHostWhenExplicitlyEnabled`，
1 passed / 0 failed / 0 skipped；证据 `/tmp/nola-mac-real-memory.Z3S4Gi/result.xcresult`。
Provider HTTP 200，输入 16623、缓存读取 10030、输出 6 token；使用正常默认窗口，
不是 16k 压缩测试。证明预算改动后普通真实调用仍能完成，不证明小窗口压缩已通过。
手动维护、服务端超窗恢复的完整请求预留，以及无 tokenizer 时的初次估计误差仍待处理。

预算门禁第一步已落地：`model_planner` 在完整投影估计及压缩返回后检查
estimated_input ≤ prompt_limit；超限不创建 InvokeModel，而记录 Core blocker
`context.input_budget_exceeded`，保留原文与验证义务，以 Blocked 封存当前 Turn。
该事实优先于 verification.pending，但不覆盖取消、执行失败或未知副作用。
`agent::runtime::requirements::tests` 8 项通过，日志
`/tmp/nola-context-budget-results.log`；其中新增测试验证受阻原因、原文保留及
验证义务未被误消除。**尚未完成摘要完整成本预留、真实 tokenizer 误差处理、
Store 纵向封存和修改后的真实模型验收，不声称窗口问题整体修好。**

**复验范围纠正：任务流程通过，严格窗口预算未通过。** 最新 provider trace 中，
带工具请求输入 16672–21605 token，高于配置窗口 16000（输入预算 9600）。
服务端未拒绝不等于客户端正确执行窗口准入。完整请求编译后的压缩重试限制绕过了
后续预算门禁，且摘要规划未保留完整请求成本；源码及整改契约见
[完整请求准入缺口](../../软件架构设计/模块/09-上下文与压缩/CONTEXT_AND_ACCOUNTING.md#完整请求准入仍存在的主线缺口)。
后续需要同时验收任务结果与实际请求预算，不能继续用这一项绿灯证明二者。

补齐真实请求计量后再次执行 Mac 主链验收，通过（1 passed / 0 failed / 0 skipped）：

- 证据 `/tmp/nola-mac-real-memory.xx14Ue/result.xcresult` 与同目录
  `runtime/provider.ndjson`。仍使用相同的 16000 窗口及真实配置模型。
- 17 次 Chat 模型请求全部 HTTP 200，17 次均报告输入用量；总输入 181417，
  缓存读取 80969，按 token 加权命中率 44.63%。这是整个验收的请求汇总，
  不是单次压缩命中率，也不是产品 UI 统计的验收。
- 探针现在读取 Chat 的 max_tokens / max_completion_tokens 和 usage，以及
  Responses 的 max_output_tokens / usage；只保留白名单数值，缺失仍为未知，
  不伪造零命中。实际 Chat 请求输出上限确认是 5600。
- 探针回归 `python3 -m pytest -q scripts/e2e/test_provider_probe.py`：9 passed。
  旧 trace 未采集的 Chat usage 不能据此追溯补零或声称未命中缓存。

同窗口 Mac 真实压缩与原文召回复验通过：

- 未改测试窗口，仍为 16000；使用当前 Test 构建运行
  `scripts/test_real_mac_compaction.sh`，runner exit 0，1 passed / 0 failed / 0 skipped。
  XCTest 实际 60.347 秒，完整 runner 测试阶段约 63.287 秒，不能把这些当单次
  压缩延迟。证据 `/tmp/nola-mac-real-memory.ykJf9M/result.xcresult`。
- 只读核验隔离 DB：context_window=16000、reserved_output=5600、prompt_limit=9600、
  auto_compact=9600、summary_target=5760；6 条 Run 全部 sealed/answered。
  测试明确要求 context.compacted 事件、原始 transcript 未丢失、通过
  conversation_recall 找回最初随机 fact；这些断言本轮实际执行并通过。
- 对照失败样本 `/tmp/nola-mac-real-memory.jRHXUt/`，修复的是准入预算而非测试
  窗口或减小 filler。当前证据为 Swift→Host→Core→真实 Provider→持久化；
  未声称产品 UI、自然满窗、真实截断续写或旧版本 Run 恢复通过。
- 同目录日志、provider trace、隔离数据库全部保留；未覆盖签名 Debug 应用。

小窗口默认输出的统一准入已实现：

- Domain 按窗口剩余空间、60% 输入目标、安全余量解析未设置的输出默认值；
  显式值保持或拒绝，不静默改写。HostOutputAdmission v6 保存配置窗口并重算
  验证解析来源。CLI 普通/摘要用相同 snapshot.resolved.tokens，16k 得到
  输出 5600、输入 9600，不再是输入 512。
- Domain 输出准入 9 项、CLI Host 准入 15 项、Host invocation 17 项通过：
  `/tmp/nola-window-default-domain.log`、`/tmp/nola-window-default-cli.log`、
  `/tmp/nola-window-default-host-v2.log`。首轮 Host 测试仍期待 4096 窗口可默认
  输出 16384，已按新契约调整，并保留 max_output_tokens=None 不伪造能力断言。
- 同一 16000 窗口的真实 Mac 测试已通过，见本节首项；新快照版本不兼容旧版本，
  不修改用户持久数据。

窗口/输出冲突的准入保护已接线：

- CLI 共用 host_output_admission::context_budget 以冻结 snapshot.resolved.tokens
  核验 W-O-S ≥ 512，否则返回带实际 W/O/S 的 context_output_budget_conflict，
  不接受启发式 reserved_output 被截小后冒充合法配置。初次 Host 构建与每轮刷新
  都经过该入口；不静默改用户输出额度。
- host_output_admission 整组 14 项通过，`/tmp/nola-output-window-admission-v2.log`。
  首轮 13 通过、1 项 metadata 测试因非阻塞 accepted socket 的 WouldBlock 失败；
  fixture 显式 set_nonblocking(false) 后通过，生产网络代码未改。原日志保留。
- 此修复仅使冲突提前、可诊断地拒绝；默认输出额度的小窗口适配与同窗口 Mac
  成功复验尚未完成，不把错误提前等同于真实任务已经能完成。尚未重建产品二进制。

历史失败基线（已由上述同窗口真实复验验证修复）：

根因补充：从失败库 checkpoint 直接读取配置为 context_window=16000、
reserved_output=15488、prompt_limit=512、auto_compact=512、summary_target=307。
不是尚未量化的尾部过长；是输出预留与窗口准入冲突被夹成 512 输入预算。
整改约束及后续实现见 CONTEXT_AND_ACCOUNTING 的窗口准入节。

- 使用 `scripts/test_real_mac_compaction.sh`，Test 配置、隔离数据库，保留全部
  诊断与 DerivedData，不覆盖已签名 Debug 应用。构建通过，实际执行 1 项测试，
  无跳过，12.648 秒失败；runner exit 65。
- xcresult：`/tmp/nola-mac-real-memory.jRHXUt/result.xcresult`；测试
  `MacAgentCapabilityEndToEndTests/testRealConfiguredModelCompactsAndRecallsMacAgentHostConversation`。
  错误为 request_rejected_before_admission / retained context leaves no prompt
  budget for a compaction summary。provider trace 已产生 5 条记录，不能把它当
  全程无模型调用，也不能据此宣称压缩已执行成功。
- 当前测试窗口为 16000；错误来自 Kernel summary_token_budget 的硬预算判断。
  下一步核对 mandatory/latest round 的保留量、输出预留和 prompt limit，区分
  可压缩旧尾部与当前不可容纳输入。不得删除硬限制或调大测试窗口冒充修复。
- 日志和运行数据库位于 `/tmp/nola-mac-real-memory.jRHXUt/`；这是 Mac runtime
  → Host → Core 的真实模型失败样本，不是 Mac 视觉验收。

当前二进制正常主线复验（恢复/空消息投影修复之后）：

- CLI + nola-service 构建成功，`/tmp/nola-mainline-current-build.log`。
- 同一 mainline-contract 会话重启新 CLI，真实 Qwen 执行全部测试、--version、
  --help，20.71 秒，3 条实际 shell 均 exit 0；Run
  a5eeb896-c024-4499-b417-0c7e83ec3d74 sealed/answered（核验回答，无代码修改）。
  独立复跑 18 项测试通过，版本 invoices 1.0，帮助包含 --encoding。
- 证据 `/tmp/nola-mainline-real.wAD00E/current-binary-check.ndjson` 与同目录数据库。
  这是最新二进制正常调用回归，不是旧错位 Run 恢复成功或 Mac 产品 UI 验收。

恢复决策投影门槛修复并整套复验：

- `unknown_effect_user_decisions...` 的失败并非需要给测试伪造正文：保留未知结果
  可在首个模型 Part 之前封存 blocked Run，消息生命周期 complete 合法，但
  projection 原先把它误当成成功答案并要求正文。现按 LC-05 分离消息结束与
  RunResult 业务结果，允许空 Parts，保留已有 Parts 的消息/Run 所有权校验。
- 最终源码 Application 会话投影测试 10 项通过；执行恢复整套 7 项通过，包括
  原先失败的 unknown 决策事务回滚、授权重试、保留受阻以及命令幂等。
  日志 `/tmp/nola-empty-terminal-projection.log`、
  `/tmp/nola-recovery-complete-suite-v2.log`。不是只重跑单个成功用例。
- 这修复当前恢复决策的合法终态投影；旧真实 Run 的 request 错位仍未修复，
  尚未重建 CLI/service 二进制或进行本补丁的 Mac 实机验收。

恢复入口身份策略整改：

- CLI 重启不再无条件重绑定所有 pending requests；冻结策略 Recoverable 走
  Resume 换号，Idempotent/RequiresRecoveryDecision/缺失策略保留原请求。
  权限重新呈现仍独立处理，未放宽 unknown 的执行资格。
- CLI 分型回归 1 项通过：`/tmp/nola-recovery-identity-test-v2.log`；
  Application 冷恢复 Resume、Retry/unknown 两项各通过：
  `/tmp/nola-recovery-resume-test.log`、`/tmp/nola-recovery-retry-test.log`。
  当前 CLI 构建成功：`/tmp/nola-recovery-identity-build.log`。
- 不能宣称全部恢复通过：首次尝试取消所有换号失败，原因是 Resume candidate
  需要新 request；已撤回该方案。整套 Application 测试还暴露未修改的
  unknown_effect_user_decisions 用例缺少助手 Core 投影而失败，日志
  `/tmp/nola-recovery-application-test.log`。需要另行修复该链路并重跑整套。
- 原真实旧 Run 已发生的 request 244/245 错位没有被自动修补；该补丁防止后续
  RequiresRecoveryDecision 被 CLI 再次提前换号，不构成旧数据恢复或 Mac 验收。

真实热分叉 → 继续工具任务 → 最终落库首次通过：

- 沿用 `mainline-contract` 会话，新进程增加输入 --encoding 与 GB18030 子进程
  测试，74.30 秒，18 项独立复跑通过，Run c113c073-3c2f-411e-82c5-b7d1fe6cbe15
  为 sealed/action_completed。阈值 22000 这轮未触发压缩，不算压缩样本。
- 下一轮仅在隔离验收进程将阈值降至 12000，要求为刚才两个参数补 README 并
  跑全部测试。真实触发 cached_request_fork，无 fallback；摘要输入 33318 token，
  cache read 32581（97.78%），输出 1834，elapsed 18839 ms；未重写、未续写。
- 摘要后模型实际编辑 README、运行测试，整轮 49.56 秒、exit 0；独立复跑 18 项
  通过，Run a02df399-4d0e-431f-b29d-6b4500203bf9 为 sealed/action_completed。
  证据 `/tmp/nola-mainline-real.wAD00E/contract-force-compact.ndjson`、数据库和项目。
  同会话三条 Run 均 action_completed，不是把摘要成功或进程退出当成完成。
- 该 97.78% 为摘要单请求命中率，不是会话加权总体；这是人为降低阈值的真实模型
  样本，不是自然满窗或截断续写覆盖。18.8 秒是当前 LAN 模型实测摘要延迟基线，
  高缓存命中不等于低输出延迟，不能据此宣称压缩速度问题全部解决。
- 旧 request 244/245 恢复错位现场仍未恢复；Mac UI 全链路仍未验证。

公开 Shell 契约修正后的真实闭环：

- 移除公开 bash schema 中无法由当前 Host 证明有序执行的 `mutate_and_validate`，
  明确 operation_kind 描述的是本条命令，不是整个修改任务。检查已修改文件用
  `validate`；修改与验证分调用。未按命令关键词改写参数，未放宽证据判定。
- 契约测试 1 项通过，CLI 构建成功：`/tmp/nola-mainline-shell-contract.log`、
  `/tmp/nola-mainline-shell-build.log`。
- 新二进制真实 LAN Qwen 会话 `mainline-contract` 用 47.90 秒完成增加 --version
  和子进程测试。模型自行选择 validate + 精确目标；独立复跑 16 项全部通过。
  原始 Run `b08cf043-9b40-4121-a00b-0442c1cb18a9` 已 sealed，持久 RunResult
  为 action_completed，不是仅凭进程 exit 0 认定完成。
- 证据 `/tmp/nola-mainline-real.wAD00E/contract-task.ndjson`、同目录数据库与项目。
  这证明本次工具接口整改的真实完成闭环；不代表旧 incomplete Run、压缩恢复、
  全部客户端或完整架构目标已经通过验收。

主线真实任务复验（停止扩张边角契约）：

- 当前 CLI 对 LAN Qwen 实际调用，新隔离会话 `mainline-current`：增加 JSON → CSV
  命令行入口，80.48 秒；同一会话下一轮增加缺失字段错误和失败不覆盖输出，90.45 秒。
  模型实际读取、编辑文件并执行子进程测试；独立复跑分别 11、15 项全部通过。
- **不能据此判定端到端通过**：两轮进程虽 exit 0，持久 RunResult 都是
  `incomplete / verification.pending`。第一轮测试命令被模型标为
  `mutate_and_validate`，随后验证证据仍 insufficient。这是主线完成判定与真实
  执行结果脱节的复现样本，应优先处理，不以放宽权限或把 exit 0 等同完成掩盖。
- 证据：`/tmp/nola-mainline-real.wAD00E/current-task.ndjson`、
  `current-followup.ndjson`、同目录隔离数据库与 `project/`。未修改用户数据库。
- 原压缩失败 Run 的只读诊断进一步发现：同一 effect 的 outbox request 为
  `host_request:245`，unknown delivery attempt 仍为 `host_request:244`，正式
  recovery inspection 因而没有目标；未授权重试、未直接改数据库。原现场保留。
- `context.compact` catalog 已改为 Recoverable（非任意自动重放）；策略测试 1 项
  通过，CLI/service 构建通过，见 `/tmp/nola-compaction-replay-policy.log` 与
  `/tmp/nola-compaction-recovery-build.log`。不声称旧失败 Run 已恢复。

摘要预算恢复接线：Host 在最终 CompactionCompleted 前检查实际摘要长度；首次
超预算沿同一物理请求追加一次完整重写指令（不是截断或降低输出额度），仍超限
则发出 compaction_summary_budget_exceeded 的 Host Failed，不再提交 Core 必拒的
完成包。两次物理请求都单独上报用量。前缀保持与重写/续写语义分开。

- CLI `--lib compaction` 40 passed、0 failed/ignored；
  `/tmp/nola-summary-rewrite-tests-v2.log`。首次用单字符重复串模拟超长摘要触发
  另一条重复输出保护，测试失败；改为编号约束文本后，重写成功和二次超限都通过。
  首次失败日志 `/tmp/nola-summary-rewrite-tests.log` 保留。
- `/tmp/nola-summary-rewrite-build.log` 构建 exit 0。
- 用真实 PTY `--resume` 选择原 mainline-real 会话（未新建会话/删 Run）；
  `/tmp/nola-mainline-real.wAD00E/resume-terminal.log` 显示仍等待恢复。只读查明
  旧 compact effect 的 delivery 已为 outcome_unknown，replay_safety 为
  requires_recovery_decision；普通恢复没有执行新模型请求。该旧 Run 尚未恢复，
  不得把 CLI exit 0 当任务完成，亦不得擅自改数据库抹掉 unknown。

热分叉 artifact 投影修复：

- 真实 `alignment-hot` 首个不匹配记录为 edit_file 的工具结果；逐字段差异仅
  `_nola_artifacts`。修复 Chat 协议归一化，在 ordinary/archive 两路径统一移除
  Host 私有字段，不忽略正文或 call ID。
- `nola_cargo test -p nola_cli --lib compaction_fork`：4 passed、0 failed/ignored；
  `/tmp/nola-mainline-fork-tests.log`。新增 artifact 例验证物理前缀保持不变，
  同时改变工具正文必须拒绝。`/tmp/nola-mainline-fork-build.log` 构建 exit 0。
- `alignment` 实际 22.62 秒成功，但 fallback 为 cache_age_limit，不能证明本修复。
  `alignment-hot` 实际 5.00 秒 exit 1：摘要超出计划预算，原 Run 未完成。
  `alignment-fixed` 被原 Run 未完成所有权拒绝，未运行摘要；不算真实热分叉通过。
  全部证据仍在 `/tmp/nola-mainline-real.wAD00E/`，不覆盖首批失败记录。

真实 LLM 主线验收（按用户要求优先验证实际任务完成，不继续扩张边角契约）：

- 当前源码 `nola_cargo build -p nola_cli --bin nola` 成功；日志
  `/tmp/nola-mainline-real-build.log`。使用用户配置的 LAN Qwen
  `qwen3.8-flash-next-iq3_s`，不是模拟模型；CLI `--yes`，独立临时项目和数据库。
- 第一轮读 README/实现/测试，修复 Decimal 发票汇总与数量校验，补边界测试，
  实际运行测试：42.33 秒，进程 exit 0；独立复跑 4 项通过。
- 退出 CLI 后在同一会话启动第二轮，追加 UTF-8 CSV 导出：82.52 秒，exit 0。
  两次真实测试失败均继续修正，最后 6 项通过，原有汇总测试保留。
- 独立断言验证 Decimal 聚合、中文/逗号 CSV 转义、排序、两位金额和非法数量；
  只读检查同库两条 Run 均 sealed，两个 RunResult 均 action_completed。
- 证据目录 `/tmp/nola-mainline-real.wAD00E/`：`first.ndjson`、`followup.ndjson`、
  对应 stderr、`nola.db`、实际修改后的 `project/` 和可复现调用 `run.py`。
  不复制密钥到证据目录，不改用户数据库。日志缓存值是单请求口径，未冒充会话总体。

这证明当前 CLI 真实读/写/执行、测试失败后继续及重启后自然追问链路；不证明
Mac UI、自然满窗压缩、手动权限等待或所有后台恢复分支。Store 当前 schema 73；
本轮 Core settlement 即时关闭接线已编译，新增专门权限回归尚未运行。

同一真实任务历史的压缩后续接验证（追加于上次两轮之后）：

- `run.py compact` 设置测试阈值 22000（不改用户配置），要求不读写文件，回顾
  两个已完成任务及失败修正；30.31 秒、exit 0，RunResult 为 answered。
- 库中 compaction_requested/completed 各 1，source fingerprint 对应；压缩 15 项，
  保留尾部 33 项。最终回答准确区分第一轮 4 项、第二轮最终 6 项测试通过，并复述
  Unicode 排序和 Decimal half-even 引起的两次失败及修正，没有把旧摘要的待办
  当成当前进度。`session.txt` 是通过 CLI session show 从持久库读回的实际回答。
- 性能未过关：本轮 3 次模型请求输入量分别 26048、4959（摘要）、27769 token，
  cache read 均为 0；摘要耗时 13366 ms。Host 明确报告
  `fallback_reason=archive_not_in_snapshot`、`strategy=incremental_summary`，不是
  已证明的 TTL 到期。应优先查原文到 wire snapshot 的范围映射，不能通过删除
  来源核验强行热分叉；压缩后输入量上升也需单独分析静态前缀/尾部/摘要贡献。
- 前两轮实际请求数 6/12，输入 112638/298191、缓存读取 40588/105016；合并命中率
  为 145604/410829，约 35.44%，不是最后一条 context_usage 的 61.23%。

仍不声称真实满窗、热分叉或截断续写已验证；本次只证明当前二分上下文的真实
摘要与保留尾部可以完成回顾，并提供了热分叉未覆盖的可复现主线样本。

Store 原 permission effect 来源校验：

- Core lib 486、Host contract 143 全通过，exit 0；
  `/tmp/nola-permission-source-binding-core.log`。
- Application `tool_batch_receipt_e2e` 10 全通过，exit 0；最终日志
  `/tmp/nola-permission-source-binding-app-verified.log`。在隔离库篡改 pending witness
  的 call_id/turn_id，真实命令返回 Rejected，原 logical request/delivery 不变且
  无新 settlement；恢复原 witness 后正常提交、故障回滚和冷重放仍通过。
- CLI `cargo check -p nola_cli --lib` exit 0；
  `/tmp/nola-permission-source-binding-cli.log`。

首次 Application 编译因新增 helper 漏闭括号失败（source-binding-app.log），
修正后两个新反例因测试误用 unwrap_err 失败（source-binding-app-final.log）；
实际 API 返回持久 Rejected receipt。最终改用独立 command ID 并检查 typed
Rejected 后通过，不覆盖首次失败记录。未验证跨端 UI 或原等待关闭/后继资格。

PermissionSettled canonical 事件链（全部代码/用例修改后统一验证）：

- `nola_cargo test -p nola_core --lib --test agent_core_host_contract`：486 + 143
  通过，零失败/忽略，exit 0；`/tmp/nola-permission-settled-core.log`。同批许可
  复用生成的两个事件逐项匹配 receipt，并带原 Turn 身份。
- `nola_cargo test -p nola_application --test tool_batch_receipt_e2e`：10 通过，
  零失败/忽略，exit 0；`/tmp/nola-permission-settled-application.log`。单次与会话
  选择经过真实 Application/Store 提交，在 command receipt 失败时无 settlement
  残留；成功后事件保留 Core envelope，冷重放与第一次提交相同、不重复追加。
- `nola_cargo check -p nola_cli --lib`：exit 0；
  `/tmp/nola-permission-settled-cli-check.log`。只证明当前 CLI 消费者编译覆盖。

未验收 Store 单项等待关闭、Task 后继资格或客户端渲染；新事件还不是完整
effect/occurrence 关闭证明，必须按 LC-05a 继续关联原持久来源。

许可复用公开 Part 一致性：`nola_cargo test -p nola_core --lib --test agent_core_host_contract`，
Core 486、Host 143 通过，零失败/忽略，exit 0；日志
`/tmp/nola-permission-reuse-projection.log`。真实 Core 批调用路径按 receipt call_id
核对所选 Tool Part 的 allow_conversation 与复用 Part 的 conversation_rule_applied，
来源/冷恢复反例保持通过。未进行客户端渲染或 Store 非终态等待关闭验收。

许可复用持久来源：`nola_cargo test -p nola_core --lib --test agent_core_host_contract`，
Core 486 通过、Host 143 通过，均零失败/忽略，exit 0；日志
`/tmp/nola-permission-reuse-source.log`。同批同主体测试从真实 Core permission
response 产生两次 ExecuteTool，但只保留一条用户 Authorization；另一条为引用
原 request ID 的 ConversationRuleApplied。冷恢复成功，并拒绝来源缺失、自引用、
倒序、AllowOnce 和 Deny 五个反例。不同主体仍需分别明确批准的回归保留。
checkpoint 当前 schema 18；没有旧版本兼容、用户数据库迁移或新 Mac 真实验收。
这不是 Store settlement/typed offers 的来源证明或完整跨端关闭验收。

LC-05a 热等待取消的 supervisor 闭环补验：扩展原取消用例，不另造停止服务。
实际 waiting callback 返回 StopRequested 后等待线程退出，调用生产
recover_orphaned_agent_tasks，验证原子封存的 RunResult::Cancelled、Task terminal
receipt、原 permission source 对应的 RunSealed closure；decision/resolved_at
仍为空。重开数据库后再次恢复，receipt/closure 不变，迟到批准返回 Closed，
Task attempt 和 child Run 数量均保持 1。日志 `/tmp/nola-permission-stop-lifecycle.log`，
`nola_cargo test -p nola_cli --lib task_runtime::permission_wait_tests`：20 通过，
零失败/忽略、1743 未选中，exit 0。仍有既有 __eh_frame 链接器警告。
这是隔离 Application/Core/Store/CLI supervisor 链，不包含实际 UI，也不证明
非终态许可复用的 settlement 即时关闭或所有取消竞争时序。

LC-05a 取消不是用户拒绝：全部代码/用例修改后统一执行，mbx、
RUST_MIN_STACK=16777216、RUST_TEST_THREADS=2。

- `nola_cargo test -p nola_cli --lib task_runtime::permission_wait_tests`：20 通过，
  零失败/忽略、1743 未选中，exit 0；日志 `/tmp/nola-permission-stop-wait.log`。
  新增真实 Core effect/隔离 Store 等待后取消：返回 StopRequested，保留用户
  decision/resolved_at 空值，Task stop intent 已提交，本地 lease 撤销且无后继 grant。
- `nola_cargo test -p nola_cli --lib sop_custom_permission_host_tests`：1 通过，
  零失败/忽略、1762 未选中，exit 0；日志 `/tmp/nola-permission-stop-host.log`。
  原 Host 传输回归增加 stop 回调：只容许已有 ExecutionStarted，不发批准/拒绝、
  failure、transport timeout 或 CancellationConfirmed；SOP 不伪造用户 disposition。

四个取消代码入口已统一，新增动态回归覆盖热等待取消，不声称穷举四处取消时序。
本轮没有真实模型/UI 验收，未完成 Core settlement 到非终态等待关闭的纵向接线。
CLI 仍报告既有 __eh_frame 超过 16MB 的链接器警告。

权限单一判定边界：移除后台 task Host matcher 对 Core wait 的自动批准，以及
Store 按相同 Host scope_rule 批准其他 occurrence 的分支；AllowTask 不再自报
grant 范围。全部代码/回归修改后顺序执行（mbx、RUST_MIN_STACK=16777216、
RUST_TEST_THREADS=2）：

- `nola_cargo test -p nola_application --test permission_request_port_e2e`：15 通过，
  零失败/忽略，exit 0；`/tmp/nola-permission-owner-application.log`。覆盖选择原子性、
  取消竞争、冷重放/撤销，以及同 Host scope 不同资源/能力仍保持 pending。
- `nola_cargo test -p nola_cli --lib task_runtime::permission_wait_tests`：19 通过，
  零失败/忽略、1743 未选中，exit 0；`/tmp/nola-permission-owner-cli.log`。新增
  host_similarity_grant_cannot_override_a_new_core_permission_wait 在隔离 Store、
  实际 Core permission effect 上预置匹配 Host grant，仍须产生原挂起记录、明确
  批准及唯一 continuation grant，不能提前返回 AllowTask。

CLI 编译仍有 __eh_frame 超过 16MB 的既有链接器警告。此组没有真实模型或客户端
UI 操作；不替代 A09 typed demands/offers/执行前资源身份复核的完整生产接线。

采样结束分类真实故障复验（2026-10-12）：重新 build-for-testing 后执行同一
Mac 流式切换测试，1024 字节断流注入、product_ui、measurement、正文流门禁
均开启。runner exit 0，1/1、零跳过/失败，最终 1 个 answered seal 与生产/UI
记录一致。新构建指纹
`d78bdac322b4a7aca436088740992d702d91dc5327088f9f3112cfd3b8839261`。
首次请求实际输出 1025 字节、226 delta 后断流、DONE=0，其 sampling end 为
incomplete（4857.33 ms）；后续请求 7890 字节、1778 delta、stop + DONE=1，
sampling end 为 succeeded（23463.87 ms）。339 个 UI 时间线样本最终 running=false。
产物 `/tmp/nola-mac-real-memory.ztgswO`，外层日志
`/tmp/nola-mac-sampling-outcome-real.log`。该样本证明正常采样路径的计量修复和
故障恢复，压缩分叉仍只有共享分类函数及模块测试证据；不代表完整成功率分母、
多协议性能预算或全部模块验收。

采样结束原因分类（2026-10-12）：经 mbx 初始化，以
`RUST_MIN_STACK=16777216 RUST_TEST_THREADS=2 nola_cargo test -p nola_cli --lib llm::`
复验，152 通过、0 失败/忽略、1609 未选中，35.60 秒、exit 0。日志
`/tmp/nola-sampling-outcome-tests.log`。新增结束原因矩阵与部分流断言通过，正常和
分叉生产出口采用同一函数；这不是 CLI 全量或新版本真实计量验收。已另行启动
重新构建后的 Mac 断流复验，见 AUDIT，尚未记录通过。

同构建真实断流恢复（2026-10-12）：复用上一条正常流验收的 Test 产物（源码未改），
同一流式会话切换 XCTest，设置 `NOLA_MAC_REAL_INTERRUPT_AFTER_TEXT_BYTES=1024`，
product_ui、measurement、正文流门禁均开启。runner exit 0，XCTest 1/1、零跳过；
恰好一次实际断流发生在 1098 正文字节、237 delta、无完成标记，随后请求输出
9020 字节、1932 delta、stop + DONE=1。生产结果与持久 seal 比较为 1 个 answered。
实际 UI 编号 1…60 无废弃前缀、底部视口、草稿隔离和按钮断言通过，时间线 344
样本且结束 running=false。产物 `/tmp/nola-mac-real-memory.xRgVcv`，外层日志
`/tmp/nola-mac-trace-owner-interruption.log`；构建指纹仍为
`20fd67ef02b1df0210a47480ca042215304a229d970775ea8aa0f1dc99ba2ce9`。
发现测量缺陷：首次不完整流的 provider_sampling end 仍标 succeeded；源码按
Ok(choice) 判定而未核对 finish_reason。因此此轮证明业务恢复，不证明请求成功率
计量正确。保留失败流样本，不能将两个 sampling succeeded 统计成两次完整生成。

当前 Mac 流所有权整改后的真实流式切换（2026-10-12）：
`run_real_mac_xctests.sh` 选择
`MacRealProductUIEndToEndTests/testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible`，
product_ui profile，开启 measurement、REQUIRE_TEXT_STREAM 与保留产物，实际
build-for-testing 后运行；最终 runner exit 0，XCTest 1/1、0 跳过/失败。
生产验证结果与持久 seal 比较得到 1 个 answered Run，正文流门禁也通过。
产物 `/tmp/nola-mac-real-memory.RmfEKJ`；外层日志
`/tmp/nola-mac-trace-owner-real.log`。隔离数据库与 Test 构建，未改用户日常数据。
构建指纹 `20fd67ef02b1df0210a47480ca042215304a229d970775ea8aa0f1dc99ba2ce9`。
测量文件记录 5 个事件，adapter 首正文相对 sampling begin 为 2290.18 ms，
sampling 13983.45 ms，HTTP headers span 36.53 ms；不是点击到首帧，也不代表 p95。
时间线 215 样本，最后 running=false。测试覆盖活动流切换、草稿/按钮隔离与最终
内容检查，不覆盖多 Turn 迟到真实网络注入、权限全部状态或所有平台视觉验收。

Mac 单流所有权（2026-10-12）：`./scripts/test_mac_layers.sh contracts` 最终
exit 0、TEST SUCCEEDED；XCTest 执行 997 项，2 跳过、0 失败，156.417 秒，另
Swift Testing 29 项通过。新增流切换回归 0.002 秒通过，实际消费入口旧 Turn
失败拒绝也通过。日志 `/tmp/nola-mac-trace-stream-owner.log`。这不是完整真实
重连或多来源验收；随后另行启动真实流式会话切换，见 AUDIT，尚未记为通过。

Mac trace 载荷冲突准入（2026-10-12）：同一次
`./scripts/test_mac_layers.sh contracts` 最终 exit 0，TEST SUCCEEDED。
XCTest 执行 996 项，2 跳过、0 失败，156.840 秒；Swift Testing 另 29 项通过。
`testTraceConflictRejectsChangedPayloadWithoutConsumingSequence` 0.001 秒通过，
覆盖同 ID 改 payload/sequence、冲突不改投影、不占序号、原重放和非法新流不清除
旧历史。日志 `/tmp/nola-mac-trace-conflict.log`。Test configuration，未覆盖签名
Debug；结果不是实际 provider 或多端 UI 验收，不证明摘要计算性能预算、去重条目
总量有界、跨 Turn 生命周期或原网络未知字段冲突处理。

Mac trace 消费入口准入（2026-10-12）：同一次
`./scripts/test_mac_layers.sh contracts` 完成，exit 0、TEST SUCCEEDED。
XCTest 执行 995 项，2 跳过、0 失败，156.558 秒；Swift Testing 另 29 项通过。
实际入口新增用例 `testRejectedTraceCannotCreateMessageOrChangeCallerResult`
0.093 秒通过，覆盖身份错配、重复、旧 generation 不创建消息/改变调用方结果，
以及合法后继事件仍可消费；权限消费整套回归也通过。日志
`/tmp/nola-mac-trace-consumer-admission.log`；Test configuration，未覆盖签名 Debug。
仍有 Rust target 空间与 headermap 警告。完整权限/浏览器/usage 副作用拒绝矩阵、
逐 trace 生命周期及载荷冲突并未由该新增用例全面证明，不替代真实模型/UI 验收。

Mac trace 身份局部校验（2026-10-12）：延续同一次
`./scripts/test_mac_layers.sh contracts` 进程，无重复启动。Test configuration、
architecture-qualified Mac destination，未覆盖签名 Debug。最终 exit 0、
`TEST SUCCEEDED`；XCTest 执行 994 项，2 跳过、0 失败，157.537 秒；Swift Testing
另 29 项、5 suites 通过。新增
`testTraceIdentityRejectionDoesNotResetCursorOrConsumeEventID` 0.001 秒通过。
日志 `/tmp/nola-mac-trace-identity.log`。构建仍有 Rust target 超 24 GiB 和
headermap 警告，不宣称零警告。仅证明 reducer 局部拒绝和合同回归，未证明
消费入口的全部副作用均受准入约束，也未执行真实 Host/visual/model suites。

终态来源收紧后的 Application/Store 集成复验（2026-10-12）：最终 10 个测试目标、
69 项通过，0 失败/忽略，exit 0。先加载 `scripts/lib/rust_build_cache.sh` 并执行
`nola_rust_build_cache_init`，再以 `RUST_MIN_STACK=16777216 RUST_TEST_THREADS=2`
运行 `nola_cargo test -p nola_application --test canonical_run_terminal_query
--test task_terminal_receipt_e2e --test task_run_binding_e2e
--test run_invocation_recovery_e2e --test run_recovery_inspection_e2e
--test run_recovery_checkpoint_identity_e2e --test nonterminal_replay_e2e
-p nola_store --test orphaned_run_recovery_e2e --test remote_sync_apply_e2e
--test conversation_sync_e2e`（命令参数合为一行）。最终日志
`/tmp/nola-terminal-application-verification-complete.log`。首轮暴露同步测试的
初始 checkpoint 缺少取消提交对应的 Core session；补齐两处 fixture 后，第二轮
又暴露恢复竞争测试将零 busy timeout 用于准备数据。现仅在竞争断言前设零等待。
生产来源/身份校验未放宽。前两份日志 `...-verification.log` 与
`...-verification-final.log` 均失败，不代表最终结果。范围为真实 Application
事务、隔离 SQLite、模拟 Host/Core 输入和同步页，不证明真实设备、provider 或 UI。

正式终态来源绑定（2026-10-12）：query 复用 journal 的 local/remote 来源验证，
增加无绑定的复制 seal 拒绝断言；远端 multi-owner 隔离库测试在冷重开、重复提交、
后续 journal 追加后读取 canonical terminal，核对原 result/digest/root。最终
`nola_cargo test -p nola_store --lib` 407 通过、0 失败、1 忽略、exit 0，日志
`/tmp/nola-terminal-source-binding-final.log`，git diff --check 通过。
中间 `/tmp/nola-terminal-source-binding.log` 不含最后加入的远端查询断言，不能
代替最终结果。本项验证已保存来源绑定和 SQLite 恢复，不证明真实网络认证或 UI。

正式终态来源唯一性（2026-10-12）：扩展 canonical terminal 隔离 SQLite 测试，
覆盖未提交 stage、相同内容双提交来源、单一但未完成清单的远端来源，以及故障
注入回滚后原封存仍可查询。`nola_cargo test -p nola_store --lib` 最终 407 通过、
0 失败、1 忽略，34.70 秒、exit 0；日志 `/tmp/nola-terminal-source-uniqueness.log`，
git diff --check 通过。该结果证明正式查询的来源拒绝规则，不证明真实远端传输、
用户 UI 或全部 manifest 校验；没有修改用户数据库。

CLI 生产完成结果传递（2026-10-12）：新增实际 CoreEventPresenter→Emitter 六分支
回归，读取 terminal_reason 并核对 JSON 的原 Core outcome；Domain as_str 与 serde
拼写逐项相等。`nola_cargo test -p nola_cli --lib terminal_` 最终 115 项通过、
0 失败/忽略、1645 项未选中、6.66 秒，exit 0；日志
`/tmp/nola-cli-terminal-outcome.log`，git diff --check 通过。范围是 terminal_ 筛选
及模拟的已提交事件，不证明真实 provider/用户操作→seal 的完整端到端，也不是
CLI 全量测试。没有把 Blocked/Incomplete 改成失败退出或自动 Goal 暂停。

共享 trace 当前版本与幂等准入（2026-10-12）：新增 schema 拒绝、同 ID 异载荷
冲突、原事件重放、第二终态拒绝及拒绝不占序号/ID 的组合回归。初次通过后清理
未使用 HashSet import，最终源码执行
`nola_cargo test -p nola_core --lib --test agent_trace_contract`：Core 484 项
（0.59 秒）与共享 fixture 1 项通过，0 失败/忽略，exit 0；最终日志
`/tmp/nola-trace-admission-contract-final.log` 无 warning/error，原中间日志为
`/tmp/nola-trace-admission-contract.log`。此为归约准入回归，不证明来源认证、
内存有界、生产消费者错误出口或真实五端验收。

Turn 完成 outcome 投影（2026-10-12）：六分支覆盖 answered/action_completed/
needs_input/blocked/incomplete/None，核对 status 与明确 reason；非成功终态收到
晚到 phase 仍保持原结果。`nola_cargo test -p nola_core --lib --test agent_trace_contract`
最终 Core 483 项（0.90 秒）与共享 fixture 1 项通过，0 失败/忽略，exit 0；日志
`/tmp/nola-trace-completion-outcomes.log`，git diff --check 通过。测试证明 Core→trace→
共享归约，不证明真实模型、canonical seal 落库和五端用户界面结果全部一致。

共享 trace 身份拒绝（2026-10-12）：新增三键身份错配与 event ID 复用共六组合，
验证先拒绝后去重、拒绝前后投影相等、后续合法序号可接受，以及 batch 对混流和
缺序号返回错误。执行 `nola_cargo test -p nola_core --lib --test agent_trace_contract`，
命令 exit 0；Core 482 项和共享长任务 trace fixture 1 项通过，0 失败/忽略，日志
`/tmp/nola-trace-identity-fence.log`；git diff --check 通过。范围是共享归约及其
fixture，不证明五端真实订阅接线或持久 Run seal 验收。

Run trace 状态所有权（2026-10-12）：新增成功/失败/取消三类终态序列，断言
工具与 hook 成败不会结束 Run，工具自己的结果仍保留；终态后 usage、phase、工具、
诊断及冲突终态不改原 status/reason/phase。最终完整源码执行
`nola_cargo test -p nola_core --lib`，481/481、0 失败/忽略、0.69 秒、exit 0，
日志 `/tmp/nola-trace-run-state-owner-final.log`，git diff --check 通过。
此前 `/tmp/nola-trace-run-state-owner.log` 是冻结 reason/phase 前的中间回归，
不能代替最终证据。本项不证明正式 Run seal 或客户端真实运行；身份混流拒绝及
冲突终态诊断仍未由当前用例覆盖。

外部 trace 生命周期隔离（2026-10-12）：附带 validation 不再替换工具结束、授权
等待及 Run 终态，独立诊断不覆盖 reducer 的运行 status。新增五类源事件 × 四类
诊断共 20 组合，核对 kind/status、附带诊断保存、后续独立诊断不改运行状态；
独立 evidence_insufficient 映射 Incomplete 另有覆盖。最终
`nola_cargo test -p nola_core --lib` 480/480，0 失败/忽略、0.61 秒、exit 0，
日志 `/tmp/nola-trace-lifecycle-isolation.log`，git diff --check 通过。这是共享
投影回归，不证明外部适配器真实事件采集、客户端全部消费或正式 Run seal 写入。

Mac 验证诊断隔离（2026-10-12，最终结果）：已完成
`./scripts/test_mac_layers.sh contracts`，日志 `/tmp/nola-mac-validation-diagnostic.log`，
同一次运行会话 20654 exit 0，TEST SUCCEEDED；993 项、2 跳过、0 失败，测试耗时
156.937 秒（不含构建），没有因等待重启测试。结果 bundle 位于 DerivedData 的
`Logs/Test/Test-NolaMac-2026.10.12_6-56-50-+0800.xcresult`。
新测试 `testValidationDiagnosticsDoNotReplaceRunStateOrBecomeCompleted` 覆盖五类
诊断 × 两种内容所有权模式，检查 Run 状态隔离与诊断清除，单项通过（0.003 秒）。
本轮只证明 contracts runner 包含的回归，不证明真实 provider/全部窗口/跨端冷恢复；
runner 排除的真实 Host 与视觉套件不计为已验收。headermap 与 Rust target 超过
24 GiB 提醒仍存在，不能称为零警告。git diff --check 通过；构建配置为 Test。

证据不足 trace 分离（2026-10-12）：Core 从未知范围/缺 Host 版本派生证据不足，
不再输出继续针对性验证动作。新增六分支投影回归，覆盖未知范围、未知版本、
已知版本，分别在 verifying/correcting phase 检查诊断状态及 next_action。
`nola_cargo test -p nola_core --lib` 最终 479/479，0 失败/忽略、0.59 秒、exit 0，
日志 `/tmp/nola-evidence-insufficient-trace.log`；git diff --check 通过。
这是共享 Core/trace 投影测试，不是 Mac/iOS/Android/Web 展示或真实运行验收；
Mac 仍有将 incomplete/blocked 映射为 failed 的消费端缺口，见 AUDIT。

可空回执持久化复验（2026-10-12）：新增跨 Application/Core/Store 测试首次确实
失败于提交 Host transition：null 被 Store 强制解为 ExecutionReceipt；原失败日志
`/tmp/nola-nullable-evidence-integration.log` 保留。修正 Store 的 Option 解析后，
统一执行 `nola_cargo test -p nola_application -p nola_store --lib`，Application
314 通过/0 失败/1 忽略（8.46 秒），Store 407 通过/0 失败/1 忽略（35.63 秒），
命令 exit 0，日志 `/tmp/nola-nullable-evidence-final.log`。新增用例覆盖显式 null
与 Unknown、业务伪造隔离、冷重开、重复 Host 响应不重复入库，以及恢复上下文。
两文件 rustfmt 与 git diff --check 通过。以上是隔离 SQLite 与模拟工具响应，
不代表真实模型、文件副作用、Mac UI 或完整目标验收；未修改用户数据库和凭据。

可选回执跨封装/协议校验（2026-10-12）：发现此前 Host-only 回归没有覆盖
validate_completion 对 Succeeded+Unknown 的拒绝。现 execution 为必填 nullable，
缺字段失败、null 无回执、显式 Unknown 原样保留；CLI 不补回执，Core 投影移除
data 中伪造保留字段。成功但无回执/未知可接受，Deferred 无回执拒绝，原有失败
检查和 NotStarted 成功矛盾仍拒绝。新增协议 round-trip/投影/完成校验及 Host
串联校验用例。最终 host-protocol 28/28、Core 478/478、CLI agent_host_driver
171/171，均 exit 0、0 失败/忽略；日志 `/tmp/nola-optional-receipt-protocol.log`、
`/tmp/nola-optional-receipt-host.log`（Host 3.72 秒）。git diff --check 通过，CLI
既有 __eh_frame 警告仍存在。当前结果是协议与 Host/Core 回归，不证明真实文件
效果和持久 evidence 消费完整纵向验收；不再引用上一轮局部通过证明跨模块正确。

Host 缺执行回执不提升 Completed（2026-10-12）：tool_result_contract 的缺省改为
Unknown，而非依据 error=false 合成 Completed；HostToolOutcome 独立不变。新增
用例核对成功/失败缺回执、业务字段伪造、模型式完成文案、显式五类 execution
state 保留。最终 `nola_cargo test -p nola_cli --lib agent_host_driver::` 171/171，
0 失败/忽略、3.92 秒、exit 0，日志 `/tmp/nola-host-receipt-unknown.log`；仍有
既有 __eh_frame 链接警告，git diff --check 通过。这不是实际文件效果或真实
provider 验收；非可选 ToolResult.execution 仍无法保留原始 None/显式 Unknown
差别，已在 MODULES/AUDIT 登记，不将本修正称为证据协议完整切换。

批准后工具结果/终态授权撤销（2026-10-12）：新增 standalone_approved_permission
fixture 的成功/失败两分支，实际 Core RequestPermission→ExecuteTool→终态
reducer，工具结果模拟。核对精确派生 grant 的存在、关闭故障回滚、撤销历史保留、
冷重开、原决定/来源和单 Run 不变。首次 standalone_ 筛选中权限用例通过，另一个
压缩续写 fixture 在 HTTP 请求读取遇到 WouldBlock；明确将接受的 socket 设为
blocking 后复验。原失败日志 `/tmp/nola-permission-approved-terminal.log` 保留。
最终 standalone_ 7/7（含压缩截断续写）、permission 133/133，均 exit 0、
0 失败/忽略；日志 `/tmp/nola-permission-approved-terminal-final.log`、
`/tmp/nola-permission-approved-suite.log`，后者耗时 28.01 秒。git diff --check
通过；仍有既有 __eh_frame 链接警告。不是实际 provider/文件副作用或 UI 验收。

无挂起权限非取消终态（2026-10-12）：新增真实 Core effect/reducer/Store 链路
fixture，用户拒绝后分别 ModelCompleted 和不可重试 Failed，验证原 Run seal
关闭原请求、保留决定/时间、关闭故障使 Task receipt 回滚、冷重开、Ensure 与
对账重放不复活/不增加 Run。定向 1/1（内部两分支），最终 CLI permission
132/132、0 失败/忽略、29.85 秒、exit 0，日志
`/tmp/nola-permission-non-cancel-terminal-final.log` 与
`/tmp/nola-permission-non-cancel-suite-final.log`。git diff --check 通过。
首次新增测试编译因两个 closure 同时可变借用 Application 失败，改为显式传入
调用者后通过。首次全量则有既有 queued-input HTTP fixture 首行读取 WouldBlock，
原 3 秒读取窗口与启动并发竞争；接受连接后显式清除 nonblocking，并用服务端
剩余期限设置读取超时后全量通过。原失败日志 terminal.log / suite.log 保留。
不是实际 provider/工具成功执行或 UI 验收；批准后工具完成/失败及撤权组合仍缺。

架构总览符合性对齐（2026-10-12，文档审计）：核对 MOD-01～15、当前 schema 72、
permission_request_sources/closed 约束、Domain 提取/冻结接口与最近权限回归记录。
README 增加六项目标→规范入口→当前事实→完成所需证据矩阵，修正权限关闭整体
仍“待实现”的过时标签，区分生产接线、纯契约与真实客户端验收；补明确 /tmp
产物丢失和不同构建证据不能合并为当前通过。新引用文件存在，git diff --check
通过。本轮未修改运行代码、未重跑测试，不将文档对齐登记成架构整体已完成。

Kernel 授权字面值与配置 pattern 分离（2026-10-12）：新增文件/JSON 中星号、
能力名星号、未知能力伪装 shell 前缀用例；Conversation 按字面或既有明确范围
解释，配置 pattern 保留。初版同时约束 Turn 导致 CLI /allow 全允许回归失败，
已按实际构造来源修正：Turn 包含显式 profile，可通配但不得泄漏到后台 origin。
旧删除授权测试改为具体文件并补其他文件 Ask，不放宽生产 matcher。
最终 Kernel 125/125、Core 478/478、CLI permission 131/131（26.25 秒），均
exit 0、0 失败/忽略。最终日志 `/tmp/nola-permission-literal-rules-final2.log`、
`/tmp/nola-permission-literal-cli-final.log`；失败阶段 literal-rules.log 和
literal-cli.log 保留，不能以中间通过替代最终结果。CLI 既有 __eh_frame 警告
未消除；git diff --check 通过。未做真实 UI/模型或完整 typed scope 接线验收。

Host 不得改写 Core 会话资源（2026-10-12）：普通/委派授权共用资源选择函数不再
读取 Host grant 字符串/resource_pattern；测试包含 `*`、根目录、对象范围覆盖，
均保留原源/目标输入。CLI 删除原资源改写目录函数，前台返回无自定义范围的决定；
终端显示测试改为完整文件路径。最终 Core lib 478/478（0.59 秒），CLI permission
131/131（29.02 秒），均 exit 0、0 失败/忽略。日志
`/tmp/nola-core-grant-authority.log`、`/tmp/nola-cli-grant-authority.log`。
CLI 仍有既有 __eh_frame 链接警告；git diff --check 通过。测试证明范围选择函数
保留原资源，不证明 Host 响应整体被拒绝，也不是原生 UI 或全链路授权验收。

Core 资源声明完整性（2026-10-12）：新增 Composite 缺项/null 回退完整输入、
多资源 built-in catalog 保留完整输入、路径字面空格保留测试；fallback 的移动
文件用例改为同时保留源和目标，不再期望只有目标。权限专项 39/39，随后
`nola_cargo test -p nola_core --lib` 全量 478/478，0 失败/忽略，exit 0，0.57 秒。
日志 `/tmp/nola-core-resource-completeness.log` 与
`/tmp/nola-core-resource-completeness-full.log`；git diff --check 通过。
这是 Core 数据投影回归，不是 CLI/Mac 实际授权或 Host 资源身份解析验收。

生产 CLI scope_rule 收紧（2026-10-12）：格式 3 精确匹配 tool，拒绝旧格式、缺
tool、缺失/空 resource 和原命令授权用于无命令调用；删除项目标记扩大目录逻辑。
修订目录复用用例，新增前台/后台能力隔离和畸形规则回归。最终命令
`RUST_MIN_STACK=16777216 RUST_TEST_THREADS=2 nola_cargo test -p nola_cli --lib permission`
exit 0：131 通过、0 失败/忽略，34.59 秒，日志
`/tmp/nola-permission-scope-tightening.log`。仍有既有链接器 __eh_frame 大于 16MB
警告；未声称零警告。git diff --check 通过。没有真实模型或 Mac UI 验收，未完成
Core 资源解析/冻结 offers/Store 选择事务的纵向替换。

资源解析结果关联校验（2026-10-12）：Domain subject_for 验证 resolution identity、
target、invocation digest、每个需求及其 access/kind/evidence ref；原始绑定不去重，
coverage subject 可去重同资源同 access。新增回归覆盖乱序响应、漏项、重复 occurrence、
跨请求/目标/输入、篡改 locator/access/kind、空证据、严格 wire 和序列化重开。
最终 Domain 单元 169/169、LLM 架构 4/4、output_token_admission 8/8、workspace
架构 16/16，共 197 项通过、0 失败/忽略，exit 0；日志
`/tmp/nola-resource-resolution-domain.log`，git diff --check 通过。没有 Host 资源
解析或 Core response 接线验收；非空 evidence_id 不等同来源或新鲜度已证明。

资源需求提取契约（2026-10-12）：Domain permission_resource_plan 提供显式 One/Each
JSON Pointer 声明，保留全部需求/数组元素的 requirement_id/index/access，缺字段、
空集合、类型错误不返回部分成功；测试包含 move 两端、数组重复路径但不同出现
位置、指针转义、未知工具/提示词不改变提取、无效声明及严格 wire。路径不 trim
或本地展开，空白/反斜杠/换行字面值保留给目标 Host；同步修正新 scope matcher
错误拒绝 POSIX 反斜杠/换行文件名的全平台假设。提取结果尚不是已验证资源身份。
最终 Domain 166 单元 + 4 LLM 架构 + 8 output_token_admission + 16 workspace
架构，共 194 项通过、0 失败/忽略，exit 0，日志
`/tmp/nola-resource-plan-domain-final.log`，git diff --check 通过。
未接入旧 HostInputNormalization 或 Core 资源解析 effect，未宣称生产授权修复。

资源解析生产接线审计（2026-10-12）：核对 HostPermissionResourceDeclaration::resolve、
HostCapabilityDescriptor::permission_subject、tool_driver 的 subject 保存、built-in
声明工厂及 CLI 范围派生，确认现有目录资源声明是字符串提取而非资源身份解析。
补齐 PERMISSION_CLOSURE 的资源解析协议（需求声明、Host 事实、Core 提交、offers、
执行复核）、状态/故障/恢复/性能/验收矩阵；明确这些新 effect 尚未实现。
本轮只有源码审计与文档变更，git diff --check 通过；没有新增生产接线测试结果。

独立 Ensure 无挂起终态验收（2026-10-12）：新增
standalone_permission_without_suspension_closes_from_its_original_run，使用实际
Core model response→permission effect，不调用 Suspend。批准/未批准两分支
均覆盖 Ensure 保存来源→Task running 时停止意图→closure 写入故障→原 Run seal
恢复→冷重开；验证未伪造用户决定、grant 已撤销、Task receipt 原子回滚、全程
无 suspension、effect 队列已清理但来源仍保留、旧选择返回 Closed、Ensure 原
身份重放稳定、仅一个 Run 和原 attempt。模型响应为测试 fixture，不是线上模型。
CLI permission 最终 129/129，0 失败/忽略，30.76 秒，exit 0；日志
`/tmp/nola-standalone-permission-terminal.log`，git diff --check 通过。无挂起的
正常完成/失败分支及跨端真实 UI 未由此用例证明，范围契约接线也仍未完成。

冻结权限候选选择契约（2026-10-12）：Domain FrozenPermissionOffers 自动包含
Once，并将候选 ID 绑定 occurrence/subject/scope；拒绝重复候选、缺 Once、不能
覆盖原对象的范围。客户端选择类型只有 occurrence/offer_id，额外 scope 字段
反序列化拒绝。新增用例核对冷序列化重放、借用原候选、输入摘要变化、effect
变化、原范围被扩大后旧选择失效。最终 Domain 单元 162/162、LLM 架构 4/4、
output_token_admission 8/8、workspace 架构 16/16，共 190 项通过，exit 0，日志
`/tmp/nola-permission-offer-domain.log`；git diff --check 通过。
此处没有 Core effect/Store/客户端接线验收，摘要不能替代来源核验；实际接线
前置（tool_driver 仍只有字符串 subject）已记入设计和审计，未宣称授权链路修复。

范围纯契约与依赖边界（2026-10-12）：新增 Domain permission_scope 六组测试，覆盖
执行目标/文件系统/能力/操作/access 隔离、目录组件边界、全部资源覆盖、显式能力
集合、路径错误、空/重复资源、规范 origin/端口、进程摘要及平台访问、严格 wire。
模块尚未接入 Core effect/Store/Host，不能认为旧 scope_rule 已整改。

全量 Domain 检查先发现 contracts→request_snapshot→contracts 循环，以及
Responses/Streaming 引用快照编排保存原请求事实；已把数据结构、脱敏 Debug、纯
retain_request 移至 contracts，更新协议与测试引用，未放宽架构门禁。随后发现
三个 Application 源码测试直接引用旧 Core，统一经过既有 adapter。中间 CLI
编译因一个测试残留旧函数路径失败，已修正后重跑，不使用先前结果证明最终源码。

最终全部 exit 0：Domain 159 单元 + 4 LLM 架构 + 8 output_token_admission + 16 workspace 架构
共 187 项通过；Application lib 313 通过/1 既有忽略；
CLI llm:: 151/151（35.58 秒）。日志 `/tmp/nola-permission-scope-domain-final4.log`、
`/tmp/nola-application-boundary-final.log`、`/tmp/nola-llm-contract-snapshot-final2.log`。
早期失败日志 `/tmp/nola-permission-scope-domain.log`、final/final2/final3 阶段日志及
`/tmp/nola-llm-contract-snapshot-final.log` 保留用于追溯；没有真实 UI 验收结论。

授权范围审计与设计（2026-10-12）：只读核对了 supervision 的范围生成/复用、
permissions_output 的规则/路径资源派生与 matcher、Application grant 构造、
Store 决定事务以及 RequestPermission effect/capture 的字段。确认 scope_rule
未被 Core 来源核验覆盖、CLI 仍拥有范围解释。已在 PERMISSION_CLOSURE 写入
typed subject/offer/scope、各模块权限、完整流程与验收矩阵；AUDIT 登记未修复。
本批只有文档更新，git diff --check 通过，没有运行或宣称新增代码测试，也没有
把此前 44/599 项回归扩大为授权范围安全验收。

首次 Run 绑定的权限边界（2026-10-12）：Task 首绑准入在预检及实际写事务都拒绝
已有 permission request，不依据可选 Core ID 认领旧权限，不删除请求或迁移来源。
新增真实 Application admission 用例：已有请求拒绝；AFTER INSERT Run 触发器在
预检后制造请求也拒绝，子 Run 和插入的请求一同回滚、Task 不变；移除故障后
原 lease 可正常绑定。既有请求则保持原样，不以“修复”为由抹掉授权历史。
最终 task_run_binding_e2e 16/16、permission_request_port_e2e 15/15、
task_terminal_receipt_e2e 13/13 全通过，exit 0。日志
`/tmp/nola-permission-binding-admission.log`。这是准入边界验收，不代表 UI 或
scope_rule 来源已完成；无 Suspend 的独立终态完整链路仍待补充。

请求所有的 Core 来源（2026-10-12）：schema 72 新增不可变
`permission_request_sources`，证据与请求原子提交，Run/effect 唯一，不随 outbox
清理删除。绑定 Enqueue/Ensure/Suspend 共用 Core effect 核验；终态关闭读取该表，
不再依赖曾经挂起过。挂起 continuation 的证据是回执校验副本，不是生命周期真相。
CLI 真实 Core 权限 fixture 新增独立 Enqueue/Ensure 伪造输入拒绝、来源 INSERT 故障
使新请求回滚、Ensure 在无 suspension 时保存来源、随后 Suspend 复用同一不可变
证据、直接 UPDATE 来源拒绝；原冷恢复/取消/outbox 清理测试仍通过。

统一复验全部 exit 0：CLI permission 128/128（52.64 秒）；Application permission
15/15、terminal receipt 13/13；Store lib 407 通过/1 既有忽略、runtime task 32/32、
permission successor 4/4。日志 `/tmp/nola-permission-owned-source-cli.log`、
`/tmp/nola-permission-owned-source-app.log`、`/tmp/nola-permission-owned-source-store.log`。
尚缺不经过 Suspend 的完整 Ensure→终态用例、未绑定请求随后建立 binding 的约束、
scope_rule 权威来源、撤销事件接线及跨端 UI 验收；未修改用户数据库。

未绑定 Task 全终态权限关闭（2026-10-12）：取消专用来源已替换为
`UnboundTaskTerminal`，schema 71，仅接受当前格式、不自动迁移用户数据库。
正式 completed/failed/cancelled 回执在同一事务关闭权限及撤销派生 grant；
保留原授权决定与时间，不将失败伪装成取消。新增六分支回归（三终态 ×
批准/未批准）逐一注入关闭写入失败，验证 Task 仍 running、无终态回执、
原请求/grant 不变；解除故障后原 lease 可提交，冷重开关闭稳定、重放不复活、
迟到新请求拒绝、旧决定不可重新授权。

最终统一复验全部 exit 0：Domain closure 4/4；Application permission 15/15、
terminal receipt 13/13；Store lib 407 通过/1 既有忽略、runtime task 32/32、
permission successor 4/4；CLI permission 128/128（32.96 秒）。日志：
`/tmp/nola-terminal-closure-domain.log`、`/tmp/nola-terminal-closure-app.log`、
`/tmp/nola-terminal-closure-store.log`、`/tmp/nola-terminal-closure-cli.log`。
独立 Ensure 的 Core 来源证明与跨端真实 UI 尚未完成；此处不是全目标验收。

权限迟到准入（2026-10-12）：Enqueue 与 Ensure 新建均在写事务内要求 Task
存在、未取消且仍可活动；Ensure 原身份重放先于新建检查，关闭记录不会复活，
冲突载荷仍拒绝。新增回归覆盖运行中停止意图、取消终态后冷重开两个阶段，
以及批准/未批准两分支，确认两入口拒绝新请求、无新 grant、原关闭回执不变。
最终复验：Application permission_request_port_e2e 15/15；Store lib 407 通过、
1 既有忽略，runtime_task_e2e 32/32、permission_successor_e2e 4/4；CLI permission
128/128（30.12 秒）。全部命令 exit 0，日志 `/tmp/nola-permission-admission-app.log`、
`/tmp/nola-permission-admission-store.log`、`/tmp/nola-permission-admission-cli.log`。
独立 Ensure 的 Core 来源证明、未绑定非取消终态关闭、跨端真实 UI 仍未完成；
本轮没有将单元/集成测试冒充真实 Mac 验收，也未改动用户数据库。

未绑定取消权限关闭（2026-10-12）：最后一处自动合成 deny SQL 已移除。取消意图
撤销派生 grant；运行中不提前关闭请求，正式 cancelled Task receipt 插入后在同一
事务核对回执内容、Task 状态/时间及无 Run binding，再写独立 closure。用户原决定
保留。修正 Domain 的错误推断：请求含 Core ID 不等于已有 Run binding，Task 级关闭
只证明权限入口结束，不声称 Core effect 已撤销。绑定 Task 仍只能通过原 Run seal。

最终统一复验：Store runtime_task_e2e 32/32、permission_successor_e2e 4/4；
Application permission_request_port_e2e 15/15、task_terminal_receipt_e2e 13/13；CLI
permission 筛选 128/128（27.85 秒），均 0 失败/忽略、exit 0。日志分别
`/tmp/nola-unbound-closure-final-store.log`、`/tmp/nola-unbound-closure-final-app.log`、
`/tmp/nola-unbound-closure-final-cli.log`。Domain closure 4/4 另通过于
`/tmp/nola-unbound-permission-closure-domain.log`。新增运行中停止→无终态/无提前关闭→
精确 worker 取消回执→重开 Closed 的批准/未批准两分支；关闭 UPDATE 抛错或被 IGNORE
均回滚 Task、suspension 与新回执，之后可正常取消。尚未覆盖未绑定非取消终态、
独立 Ensure 来源/迟到创建及跨端 UI，不将这些结果扩大为整个权限模型验收。

绑定 Run 权限关闭生产接线（2026-10-12）：schema 70 的 closure_json/withdrawn 和
关闭后不可修改约束已接入统一读写；原 seal 对账核验持久来源、写 closure，与
grant 撤销、suspension 关闭和 Task receipt 同事务。保留用户原决定/时间；Get
可读，Pending 查询移除，Authorize/Intervene 优先返回 Closed，不因 cancel flag
仍为真退化成永久 StopPending。CLI 不消费已关闭请求中的历史允许，热等待/公平
执行等待遇到关闭退出，不重新取得执行权。没有自动迁移或改动用户数据库。

证据：CLI permission 筛选 128/128 通过（43.48 秒），日志
`/tmp/nola-permission-closure-cli.log`；补回执 Debug 脱敏后最终 permission_wait_tests
15/15 通过（9.02 秒），日志 `/tmp/nola-permission-closure-cli-final.log`。实际 Core
权限取消场景注入 closure UPDATE 故障，验证无 Task 回执、挂起/请求不变；解除
故障从原 sealed Run 恢复。新增先 AllowConversation 再取消，核对决定保留、关闭
来源 seal、关闭后不能授权、重开及重放不变、SQL 修改关闭历史被拒绝。

Application permission_repository_e2e 2/2、permission_request_port_e2e 14/14、
task_terminal_receipt_e2e 13/13 通过，日志 `/tmp/nola-permission-closure-app.log`。
Store lib 407 通过/1 忽略（65.34 秒）、permission_successor_e2e 4/4、runtime_task_e2e
32/32 通过，日志 `/tmp/nola-permission-closure-store.log`，命令均 exit 0。这些耗时
不是性能基线。未绑定取消仍有旧 deny 写点；Ensure 来源和跨端 UI/真实模型验收
尚未完成，不将本批绑定路径通过写为全部完成。

权限来源原子捕获（2026-10-12）：绑定 Task 挂起事务核对当前 Core witness 中唯一
pending permission effect、不可变 payload 摘要及实际参数/指导；Store 生成类型化
来源证明随 suspension 保存，拒绝 Host 注入。热路径回执保持原意图相等，仅接受
Store 增补的精确来源。冷恢复/取消/等待权限预算停止 fixture 均改实际 Core 请求；
覆盖伪 Core ID、换能力/参数、伪来源和原绑定三项串用，失败不写请求/状态/事件。
取消用例验证原 effect 已被队列清理且重开仍保留完全相同的来源记录。

最终 CLI lib permission 筛选 127/127 通过，0 失败/忽略，30.66 秒（1625 未选中），
日志 `/tmp/nola-permission-source-capture-4.log`；Store runtime_task_e2e 32/32
（1.72 秒）、permission_successor_e2e 4/4（0.17 秒）通过，日志
`/tmp/nola-permission-source-store.log`，均 exit 0。首轮发现测试误用 CLI Store 包装
类型；第二轮 13/14 暴露热回执逐字节比较；第三轮 126/127 暴露旧测试期待批量
合成 deny。分别修正类型、严格的增补回执校验、孤立权限不伪造决定的契约断言后
复验。未在线调用模型、未验收 Mac UI；Ensure、独立 closure/schema/终态消费仍未
完成，不将来源保留冒充用户请求已经以正确关闭状态落库。

权限挂起绑定仲裁（2026-10-12）：Store 在原挂起事务、请求 INSERT/租约撤销前核对
已有 Task Run binding 的 Run、child、input watermark。CLI 真实 Core 权限来源
用例新增三个串用反例，核对请求/suspension 不存在、Task/事件不变，随后原正确
请求仍能挂起并继续。Store runtime_task_e2e 32/32（1.41 秒）、permission_successor_e2e
4/4（0.15 秒），CLI permission_wait_tests 14/14（8.14 秒）通过；命令 exit 0，日志
`/tmp/nola-permission-binding-store.log`、`/tmp/nola-permission-binding-cli.log`。
范围仅为绑定身份，不证明 Core permission ID/effect 已在写事务验证或永久保存。

删除公开批量拒绝旁路（2026-10-12）：移除 DenyPendingForTask 以及 CLI、Application、
Store port、Store 对应方法，stop 不再额外生成用户拒绝。协议负例拒绝旧命令，
保留逐请求 Authorize/Deny。Application permission_request_port_e2e 14/14、
permission_repository_e2e 2/2 通过；CLI permission_wait_tests 14/14 通过，8.07 秒，
命令 exit 0。日志 `/tmp/nola-remove-bulk-denial.log`、
`/tmp/nola-remove-bulk-denial-cli.log`；源码检索无该 Rust 入口残留。内部未绑定
取消和 Run seal 对账的两个合成 deny 写点仍待完整 closure 替换，不能宣称已完成。

权限关闭共享契约（2026-10-12）：新增 Domain 的独立 closure、闭域来源、Core
occurrence 和原请求结构校验，用户决定不放入关闭记录。目标测试 4/4 通过，0
失败/忽略，命令 exit 0；日志 `/tmp/nola-permission-closure-domain.log`。覆盖身份
串用、无效时间、将 Core 请求伪装未派发取消、无效摘要、缺字段/未知字段和断线
伪来源。结构校验不验证持久来源；Store/schema/关闭事务与 UI 尚未接线，生产
终态中的合成 deny 问题仍未修复，不能用这些单测替代纵向关闭验收。

真实 Core 权限来源（2026-10-12）：CLI 热等待用例不再硬编码权限 ID，而由已领取的
模型 effect 经正式响应提交产生工具调用和 RequestPermission；核对未授权前无
ExecuteTool、持久请求/continuation 的 Core ID 与参数一致，并取得精确后继授权。
最终 `task_runtime::permission_wait_tests` 14/14 通过，0 失败/忽略，7.81 秒，exit 0；
日志 `/tmp/nola-real-core-permission-suite.log`。本批是测试证据加强，无生产行为改动；
确定性模型响应不等于在线模型测试，冷权限关闭及 typed closure 仍未完成。

取消/授权批次共享层统一复验（2026-10-12）：本批生产改动收口后执行 Store 完整
lib：407 通过、0 失败、1 忽略，34.80 秒；CLI lib 的 permission 筛选套件：127
通过、0 失败/忽略，27.57 秒（另 1625 未选中），两命令 exit 0。日志分别
`/tmp/nola-permission-store-full.log`、`/tmp/nola-permission-cli-full.log`。覆盖既有
授权解析、范围事务、Host 权限恢复、Task 取消/等待及 Web 权限配置回归；筛选套件
不等于完整 CLI，也不等于真实 Mac/移动端 UI 或未实现 closure 协议的验收。

派生 grant 写入所有权（2026-10-12）：通用 upsert 的 ON CONFLICT 会清空 revoked_at，
已改为拒绝请求派生保留 ID；正式决定事务改首次 INSERT，重放走原回执路径。新增
“请求尚不存在时预占”和“取消撤销后复活”拒绝断言，原授权仍可由正式事务创建。
完整 permission_request_port_e2e 13/13 通过、0 忽略，0.77 秒，exit 0；日志
`/tmp/nola-grant-writer-ownership.log`，限定 diff-check 通过。未完成独立 typed
provenance/closure 持久模型或跨端 UI，不扩大本批验证范围。

请求派生 grant 生命周期（2026-10-12）：绑定 Task 取消意图与原 Run seal 对账均在
各自原事务撤销精确请求派生的 grant，不删历史、不覆盖批准决定，不影响独立同 Host
grant。增加撤销 trigger 失败→Task 取消也回滚、移除故障后重开保留原决定且只剩
独立授权的反例；并发测试改为批准先赢时仍保留 grant 历史，但后续取消必须撤销。
Application 两个目标 permission_request_port_e2e、task_terminal_receipt_e2e 各 13
项通过、无失败/忽略；日志 `/tmp/nola-task-grant-revocation.log`。完整权限 closure、
通用 upsert 的来源保护及 seal 收口路径的单独 grant 正向验收仍未完成。

取消/范围授权双连接竞争（2026-10-12）：新增 8 次隔离 SQLite 实例的双线程 barrier
并发提交，经正式 Application RequestCancel/Authorize 进入独立 Store 连接，重开
核对结果。允许先提交则决定与单个 grant 同时存在；取消先提交则 StopPending、
原 pending 无决定、无 grant。两者都只保留 Task 取消意图，无伪造 terminal receipt。
完整 `permission_request_port_e2e` 12/12 通过、0 忽略，0.73 秒，exit 0；日志
`/tmp/nola-permission-race.log`。不声称这 8 次调度必然覆盖两种胜者，也不据此声称
此前允许的 grant 已关闭；后者仍依赖未完成的 PERMISSION_CLOSURE 接线。

权限决定与取消事务仲裁（2026-10-12）：正式 Store resolve 在同一写事务先检查取消
意图，返回类型化 StopPending，不写决定、指导或 grant；CLI 不消费为许可成功。
Application `permission_request_port_e2e` 11/11 通过、0 忽略，0.44 秒；新增隔离
绑定/取消后重开用例分别尝试 allow_once、allow_conversation、deny、provide_instruction，
核对原请求不变、无 grant、原取消时间不变。日志 `/tmp/nola-permission-stop-arbitration.log`。
CLI lib check 通过，11.30 秒，日志 `/tmp/nola-permission-stop-cli.log`。这不覆盖
并发线程仲裁、真实 UI 的 stop_pending 展示或最终 closure；相关契约仍未完整接线。

CLI 原 Run 取消恢复（2026-10-12）：停止入口不再吞取消提交错误或替绑定任务伪造
授权拒绝。原权限挂起取消测试改走真实 Application/Core/Store 准入、记录意图、
重开 Store、supervisor 恢复、原 Run seal 和 Task receipt。首轮拒绝旧夹具缺失的
冻结 Host binding（0/1，日志 `/tmp/nola-agent-cancel-recovery.log`）；补齐正式
准入配置后 1/1 通过，0.41 秒，日志同路径 `-2.log`。核对原 cancellation_id、
原 attempt/input watermark、重复恢复原 receipt 不变、只有一个 Run/attempt、
零 effect delivery；不是仅匹配 sealed 字符串。随后完整 permission_wait_tests
14/14 通过，0 失败/忽略，8.07 秒，日志 `/tmp/nola-permission-wait-suite.log`。
此取消用例是同进程重开 adapter 的真实状态链，不是杀进程后取消，也不是 Mac UI
或真实模型验收；套件中原有子进程终态恢复不能冒充本取消路径的杀进程覆盖。

取消意图与权限恢复边界（2026-10-12）：扩展真实 SQLite scheduler 集成，分别固定
“批准后取消”和“取消后批准”两种提交顺序，关闭/重开 Store 后尝试普通 fair claim
及精确 permission continuation resume，均拒绝；原 Task、suspension、attempt 不变，
allow_once 决定未被取消伪造为 deny，也没有终态 receipt。完整 `nola_store`
`runtime_task_e2e` 32/32 通过、0 失败/忽略，1.42 秒，exit 0；日志
`/tmp/nola-bound-cancel-permission.log`。这是通用 scheduler fixture（shell_process）
加保留 binding 的 Store 边界反例，不是生产 agent admission、并发线程竞争、真实
进程杀死恢复或 UI 授权验收；不能替代上层完整链路。

绑定 Task 取消意图与恢复（2026-10-12）：Store 绑定分支只持久取消意图，六种活动
状态均保留原状态/attempt/结束时间，不制造 receipt；重复请求不刷新意图，重开保持。
CLI recovery 将所有已请求取消的绑定 Task 交原 Run cancel-only，不限 waiting_input。
Application `task_terminal_receipt_e2e` 13/13 通过、0 忽略，3.55 秒，日志
`/tmp/nola-bound-cancel-intent.log`。CLI check 首轮 E0308（诊断函数返回 String，
match 其他分支返回 unit），修正语句返回后第二轮通过，6.05 秒，日志
`/tmp/nola-bound-cancel-cli-check-2.log`。限定 diff-check 通过。尚未跑 CLI 运行时
冷取消或 Mac/移动端真实交互，不能将 Store 回归和编译通过等同于全链路取消验收。

Task 终态来源事务保护（2026-10-12）：共用 receipt writer 拒绝已有 Run binding
却非 BoundRunTerminal 的终态；覆盖普通收尾、expiry、带事件 expiry、queued cancel、
expired cancel、retry cancel 六入口。Application `task_terminal_receipt_e2e` 完整目标
12 通过、0 失败/忽略，2.90 秒，exit 0；日志 `/tmp/nola-bound-terminal-guard.log`。
新增反例检查回滚后的 Task、attempt、事件计数、无 receipt 和重开一致性。未运行
全库、CLI/Mac 取消纵向；取消意图与原 Run 收尾的全部客户端接线尚未完成，详见 AUDIT。

中央终态门禁统一（2026-10-12）：event_terminal 不再由 Trace doneCount 推断正式完成，
改同一步骤的生产终态读回与只读 Store seal 精确比较。保留六种结果分类，生命周期
封存通过不等于成功任务；Mac 正向门继续只接受 answered/action_completed。
judge/coverage 26 项、seal 比较器 5 项全部通过；含 Trace-only 拒绝、跨步骤拼证拒绝、
六种结果逐一计数、foreign Run 拒绝。只读复核已有真实 Mac `HvCxlT` 的 transcript 与
`runtime/ApplicationSupport/Nola/State/nola-v4.db`，无 Trace 输入，得到 1 sealed/answered、
0 failed，中央门通过。限定文件 diff-check 通过。本批没有重新发真实模型请求；其他
平台必须补同等导出链路，且各场景仍需独立约束预期 outcome，不能以生命周期门代替。

Provider trace 单一统计实现（2026-10-12）：删除生成器/判定器各自解析的重复逻辑，
统一覆盖 Chat/Responses/Messages 生成路由，区分已观测请求、HTTP 成功和完整流。
修复“本地模型成功＋远端模型发现”拼凑成远端模型验收的误判；坏行/不可读文件门禁
失败。新增 4 项 unittest、Mac receipt 3 项、既有 judge/coverage 24 项全部通过；
shell 语法通过。只读重算既有真实 Mac `HvCxlT/runtime/provider.ndjson` 得 HTTP 成功 2、
模型请求 1、完整模型流 1、坏行及不可读文件 0；没有新发模型请求或重跑 Mac UI。
完整提交分母、不同文件副本去重及其他协议流终止证据仍不在本批覆盖内。

Mac XCTest 回执证据归属修正（2026-10-12）：runner 保存 xcresulttool 原始 summary，
回执中的用例执行/通过/未跳过断言再次核对该 summary，并引用它而不是 provider trace。
请求数量读取既有 provider trace 统计，不再用 expected test count 强制至少一条请求。
3 项脚本回归通过（包含一用例多请求、仅模型发现计零、缺字段/失败/跳过/数量不符拒绝），
shell 语法和限定文件 diff-check 通过。本批仅修验收脚本，没有重新运行 Mac XCTest。
当时统计器只识别 Chat Completions 和结构化 external executor；路由识别已由上方
后续批次统一，完整请求分母仍未具备。

真实 Mac UI＋正文流验收（2026-10-12）：审计发现 runner 仍以旧 completed/Parts/Trace
判定完成。现由 Swift 调用正式 canonical_run_terminal 导出精确终态，Python 只读
同一 SQLite snapshot 对照 seal，不重写 Rust 的 manifest 验证。移除“文件存在即
Markdown/前缀/渐进流已验收”和“所有回答必须调用工具”的假断言。比较器 5 项测试
通过；shell 语法及 diff-check 通过。
隔离 Test fresh build 后第一次真实
`MacRealProductUIEndToEndTests/testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible`
1/1 通过、0 跳过，新 seal 门禁通过，但 runner 因遗留 Trace 文件为空退出 1，产物
`/tmp/nola-mac-real-memory.LHbvB4`。移除该第二终态权威残留，仅修改脚本，复用同一
构建重跑真实测试：1/1 通过、0 跳过，seal 比对通过、正文流独立门通过、runner exit 0，
产物 `/tmp/nola-mac-real-memory.HvCxlT`。测试实际切换活动会话，核对草稿、停止/发送按钮、
多次 AX 更新、60 个编号、最终标记和 footer/document 底部重排。没有修改用户 Debug
应用或用户数据库。此用例不是全 Mac UI、工具、压缩或跨端验收。
正式流观测：2352 个正文 delta、10648 字节、stop、1 个 DONE；Host 首正文回调
2266.620125 ms，sampling 30139.436333 ms，headers 30.293417 ms。测量 build digest
`f6a13e95a12f10bdd245c6bd22535c131e7c868f7d15bf69264b11b6bf12245d`。
这是单次 adapter 计时，非网络 TTFT/UI 首帧或性能 SLA；无提交前实验清单/完整回执分母，
不推导 p95、缓存命中率或端到端预算。细节见 MEASUREMENT。

Responses semantic idle（2026-10-12）：解码循环独立为生产共用 reader，以距上次有效
正文/思考/工具进展的剩余时长等待，不再因注释心跳、空 delta 或生命周期闲聊反复
重置整个 idle 窗口。新增有界通道连续闲聊用例，检查无内容超时、部分正文/思考保持
stream_incomplete、未提交工具拒绝且不改变用户取消标记。首轮因测试误用没有 Display
的 ProviderAttemptError 而编译失败（exit 101）；仅改测试为精确枚举匹配后，整个
`responses_contract_tests` 15 通过、0 失败、0 忽略，0.44 秒，exit 0。包括已有本地
HTTP SSE 正常终局/字段及中断边界用例。日志 `responses-semantic-idle-{1,2}.log`，
目录 `/tmp/nola-final-verification.gMkJRJ/`。未运行完整 CLI、真实 provider/Mac，
未验收取消后的 socket/线程回收，未统一各协议窗口数值。既有链接警告仍在。

HTTP 显式禁用超时（2026-10-12）：修复 `stream_client_builder_with_timeout(None)`
仍继承 reqwest blocking 默认 30 秒的问题，统一显式传入 `.timeout(timeout)`。
配置解析独立为纯函数，保留原有值语义；连接和协议空闲限制未关闭。源码/文档静态
审查后运行 `nola_cargo test -p nola_cli --lib stream_lifetime_tests --quiet`：5 通过，
0 失败、0 忽略，32.02 秒，exit 0。含真实本地 HTTP 头返回后正文阻塞 32 秒仍完整
读取的回归，及原有头超时、读取停顿、跨总时长三项边界。日志
`/tmp/nola-final-verification.gMkJRJ/http-disabled-timeout-1.log`。未运行完整 CLI 库或
真实 provider/Mac，未证明取消后底层 reader/socket 已回收；既有链接警告仍在。

Shell 最终观察原子提交（2026-10-12）：Application 共享 progress 校验，Store 将最终
metadata CAS、lease 撤销、终态、事件与 receipt 合并到同一事务；CLI 去掉被忽略的
独立最终 progress，只有提交成功才删除原始退出证据并写 status 文件。静态审查后
统一运行完整 Application＋CLI＋Store 库及 `task_terminal_receipt_e2e`：分别 313 通过
/1 忽略（8.03 秒）、1745 通过/4 忽略（136.14 秒）、407 通过/1 忽略（34.90 秒）、
11 通过（2.02 秒），全部 0 失败，两个命令 exit 0。日志：
`/tmp/nola-final-verification.gMkJRJ/shell-final-observation-1.log`。
覆盖实际 SQLite metadata/update CAS 冲突、事件插入故障整体回滚、正式 Application
资源/输出倒退与保留键拒绝、Shell 真实进程执行后提交故障保留原始证据、冷读成功
终态及资源 metadata。已有 `__eh_frame` 链接警告仍在。未覆盖真实 Mac/provider、
Agent seal 后晚到计量账本、未知提交结果的 observation 重放、OS 完整计量或跨 attempt
资源累计；以上不得因本批通过而标记完成。未覆盖签名 Debug 应用或用户配置。

资源采样事实契约（2026-10-12）：源码审计确认跨 attempt 累计前还存在采样真实性、
独占作用域和最终 ACK 丢失问题，详见 HARNESS_CONTROL_SCHEDULING 的 CPU/资源计量
目标契约。Unix 采样解析不再把缺损/非数值/溢出补零或饱和；拒绝重复 PID 与不完整
快照，保留合法真实零。统一回归 `nola_cargo test -p nola_cli --lib resource_supervisor_tests
--quiet`：9 通过、0 失败、0 忽略，exit 0（1.14 秒），含本机实际 ps 采样。日志
`/tmp/nola-final-verification.gMkJRJ/resource-observation-contract-1.log`；既有链接警告仍在。
此轮未验证 Windows、退出子进程累计、PID 出生身份、Shell 观察错误处理或跨 attempt
水位账本，也未运行完整 CLI 库，不得外推为预算计量完整验收。

等待授权预算观察批次（2026-10-12，实现后统一验收）：Application 新命令只接收
Task/suspension 身份、版本及时间，以原持久 scheduling/start 计算预算；Store 原子
核对原等待、attempt、取消状态及当前 Run witness，仅写首次耗尽事实和事件。CLI
恢复在权限等待短路之前消费该事实，再走原 Run cancel-only。未耗尽/已有有效 sticky
只读，存储失败与 CAS 冲突不当作成功。新增 Application 边界、实际 SQLite CAS 竞争
及 CLI 等待→观察→原 Run seal→Task receipt 回归，测试尚未执行。
静态审计发现当前 delivery witness 原本没有 active_turn 状态，现同时补齐 SQL
投影与增量编码及一致性测试（无 turn、实际 Core 准入后、Cancelling）。投影只含
turn_id/status；缺失或不认识的状态不授权预算写入，不从旧 baseline 回填兼容事实。
源码整体完成并检查后，统一 Application＋Store＋CLI 库首轮：Application 312 通过、
1 失败、1 忽略；CLI 1743 通过、0 失败、4 忽略（134.92 秒）；Store 406 通过、
0 失败、1 忽略（35.06 秒），exit 101。日志 `permission-budget-observation-1.log`。
唯一失败为 witness 测试的真实 Core submit 未配置 model.stream catalog；已补正式
fixture 能力目录和模型，不修改生产准入。第二轮完整 Application 库 313 通过、0 失败、
1 忽略（8.12 秒），exit 0，日志 `permission-budget-observation-2.log`；CLI/Store
生产源码及其测试自首轮通过以来未改。所有日志均在 `/tmp/nola-final-verification.gMkJRJ/`。
触及文件 diff-check 通过。此证据覆盖实际 SQLite CAS 与 CLI 已绑定 Run 的非执行等待
收尾、SQL/增量 witness 真实序列化一致性；不代表真实 provider/Mac、运行中独立副作用
停止，或取消/封存与预算观察的真实多进程并发压力验收。

预算停止收尾批次（2026-10-12）：初始/启动前预算拒绝及孤儿恢复接入原 Run cancel-only
通道，稳定取消身份与 typed Task budget_failure 分开保留，恢复不再 claim 普通模型
执行来处理已持久耗尽的任务。未绑定 Run 和未知副作用停止状态仍不伪造终态。
新增范围是已绑定 Run 的预算停止→原 Core seal→Store Task receipt→重开幂等核对；
生产代码不再把预算停止原因写成用户取消。扩大到完整 CLI 库复验；首轮
`task-budget-stop-1.log` 因新增测试 pattern guard 移动 String 编译失败，已改借用。
第二轮 `task-budget-stop-2.log` 完整 CLI 库：1740 通过、2 失败、4 忽略，exit 101。
新增预算 fixture 因缺少正式冻结 Host binding 被冷恢复拒绝，已使用现有正式测试准入
helper 补齐 binding，不降低生产 identity 校验。另一项 Skill 冷重开失败发生在目录
清理，残留仅 nola.db-shm：Store 的 archive worker 最后 owner 关闭只断开通知通道，
未等待线程释放连接，产生延迟 SQLite 文件竞争。现由最后 Worker owner 断开 sender
并 join；新增双 owner 生命周期与已有外部发布锁持有时同步关闭断言。第三轮扩大为
Store＋CLI 完整库：CLI 1741 通过、1 失败、4 忽略；Store 404 通过、0 失败、1 忽略，
exit 101。剩余失败发生在预算停止封存成功后的测试断言：它错误要求已被 Core 消费的
acknowledged outbox 行重开后仍存在，而正常 Store 会清理这些派发队列项。改为检查
保留队列项身份/digest（若仍存在）及必须持久存在的 delivery attempt：请求与 Run
身份不变、无 worker claim、无 execution start、无新增派发，同时保留封存及冷重开
receipt 幂等断言。第四轮 CLI 1741 通过、1 失败、4 忽略；Store 404 通过、0 失败、
1 忽略，exit 101。该 fixture 的原模型 effect 未派发，因此根本没有 delivery attempt；
“必须存在 attempt”的断言也不符合协议。最终断言以持久 Host inbox 中指向原 effect 的
CancellationConfirmed、原模型派发次数始终为零、原 Run/Task receipt 冷重开一致为准。
第五轮预算停止用例通过；CLI 1741 通过、1 失败、4 忽略，Store 404 通过、0 失败、
1 忽略，exit 101。唯一失败为此前通过的 active_stream_outlives_blocking_http_timeout，
read_to_string 返回 TimedOut：80ms 周期发送/200ms 读取窗口依赖宿主调度。该测试改为
首块确认、跨总时限、通道释放并确认尾块写入，再读取完整内容；不修改生产 timeout，
独立 stalled-body/header deadline 测试保留。第六轮完整复验通过：CLI 1742 通过、
0 失败、4 忽略（161.15 秒）；Store 404 通过、0 失败、1 忽略（47.19 秒），exit 0。
日志 `/tmp/nola-final-verification.gMkJRJ/task-budget-stop-6.log`；两次冷恢复子进程
也通过。触及文件 `git diff --check` 通过，既有 `__eh_frame` 链接警告仍在。
该批覆盖尚未派发模型 effect 的预算停止，并不等于运行中模型停止或真实 Mac/provider 验收。
附带静态审计发现 `stream_client_builder_with_timeout(None)` 未显式 `.timeout(None)`，
仍保留 reqwest 默认读取限制；此历史待办已由本页最新“HTTP 显式禁用超时”批次修复并定向验收。

副作用启动准入批次（2026-10-12）：Shell spawn 前与 Agent 执行入口前新增正式
progress CAS 观察，消费预算/取消结果及单调剩余时间；Application 允许只观察时间的
progress，仍保持全部租约校验。Shell 启动后 ACK 失败不再伪造 command-start failure，
Agent 监控线程启动失败不再降级成无监控执行。
新增真实隔离 Store 测试检查：成功 CAS 后准备时间耗尽不产生哨兵副作用，取消使 CAS
拒绝且租约水位不变，持久墙钟耗尽下原 pending 模型 effect 不被派发，Task 不被伪封存。
首轮 `task-effect-admission-1.log` 编译失败：测试混用了 StoredRuntimeTask 与
Application RuntimeTask；改为通过 CLI Application Store 读取。第二轮日志同前缀 `-2`：
Domain 4/Application 21 通过，CLI 86 通过、1 失败、1 忽略；失败为已有 mode 3 冷恢复
测试后继租约在准备阶段过期（续租入口正确拒绝）。该 fixture 用 1 秒租约验证接管，
之后却省略了生产启动续租并进行多轮 Store 反例。已在最终后继持有期间加入生产
AgentTaskLeaseHeartbeat，保留前任真实过期与接管断言。第三轮 `-3` 中 Domain 4/
Application 21 通过，CLI 83 通过、4 失败、1 忽略；四个冷恢复用例一致暴露生产权限
消费路径直接续至 now+45s，无法兼容已被 heartbeat 延长到更远时刻的同一有效租约。
该调用现复用共享 refresh_lease_before_execution：覆盖所需区间则不写，必要时才
严格延长；过期仍拒绝，不放宽 Application 的租约规则。第四轮 `task-effect-admission-4.log`
统一结果：Domain 4、Application 21、CLI 87 通过，0 失败、1 忽略，两次冷恢复子进程
通过，exit 0；既有 __eh_frame 链接警告仍在。日志目录均为
`/tmp/nola-final-verification.gMkJRJ/`。此证据包含真实隔离 Store/生产恢复函数，不含真实
provider 调用或 Mac GUI，未证明启动前耗尽的完整 Run 收尾和跨 attempt CPU 累计。

执行预算裁决接线（2026-10-12）：Domain 新增共享 execution_budget_failure，Application
进度写入复用该裁决；Shell/Agent 消费初始进度 ACK 后检查预算，恢复监控仅使用原 Task
起点的剩余墙钟，不补造未配置的墙钟限制。新增 helper 回归覆盖原起点、达到上限、
时钟倒退、缺起点、无 sticky 但 CPU 已超限及无墙钟限制。整批源码/静态审计后运行
`nola_cargo test -p nola_domain -p nola_application -p nola_cli --lib task --no-fail-fast --quiet`：
Domain 4、Application 21、CLI 85 通过，0 失败、1 忽略，两次冷恢复子进程通过，exit 0。
日志 `/tmp/nola-final-verification.gMkJRJ/task-budget-decision-1.log`。仍有 mbx 历史输入路径
预测警告及既有 __eh_frame 链接警告；未修改用户签名 Debug app。
此轮不是完整真实 Mac/provider 验收。准备阶段耗尽后的启动窗口、Shell 第二次 ACK
消费以及跨 attempt CPU 水位/不可观测恢复仍未完成，见 HCTRL-06B。

当前检查点：scheduled cold resume 来源隔离与 sealed 精确读回均已修复并验证。
Rust 协议／来源／冷恢复 27+2+18 通过；最终共享层 invocation 8/8、checkpoint 6/6、
inspection 3/3 共 17 项通过，workspace all-targets 检查通过。Mac 第四批真实 helper
12/12 通过、零跳过，两种来源冷恢复保持原权限、Run、冻结 digest 与文件副作用证据。
真实 Mac canonical-answer smoke 1/1 通过、零跳过；它不证明 provider 正文流。
独立正文流门在真实工具回答样本上正确拒绝，详见 MEASUREMENT。相关日志和各轮失败收敛记录见下文。
这不是全目标完成声明：真实多工具／压缩截断续写、UI 性能与完整缓存基线仍需验收，
CLI 历史一次冷恢复子进程 exit 2 仍未获得根因，不能因后来通过便宣称已解决。
以下时间较早的“执行中／待测”记载保留为历史记录，不覆盖此检查点。

用户配置修复后的独立最小真实 Host 调用已通过：原生加载配置、真实模型响应、
canonical 正文 NOLA_CONFIG_OK 和 Answered 终局一致，exit 0，11.183 秒。
使用隔离 NOLA_HOME／DB，临时密钥配置副本已删除；它不是 Mac UI 或完整性能验收。
Mac 真实模型 canonical-answer smoke 随后已串行通过，产物 `/tmp/nola-mac-real-memory.Uk4Oww`。

### 当前未关闭的纵向门（历史通过记录不能覆盖）

2026-10-12 验证控制归属清理：删除无合法 true producer 的工程
`completion_retry_requested` checkpoint 字段及 planner/tool driver 的 validate-only
强制执行链；保持空响应协议恢复、实际版本化验证证据和验证工具提升。
模型 advisory 同步允许诚实 Incomplete，不把待验证自动当外部 Blocked。
隔离环境统一 `nola_cargo test -p nola_core --lib --test agent_core_host_contract
--no-fail-fast --quiet` exit 0：Core 475/475、Host 契约 142/142，均零失败、零忽略；日志
`/tmp/nola-final-verification.gMkJRJ/validation-owner-cleanup-1.log`。
覆盖序列化无旧控制字段、6 类证据提示、显式用户等待/协议恢复优先级，
以及审查接受但证据不足不续跑、审查继续时写工具可用。此项不是多端真实 GUI 验收。

| 门 | 已有证据 | 仍缺的决定性证据 |
|---|---|---|
| 终态与恢复 | 共享 seal 精确读回、Mac 两种来源冷恢复通过 | CLI 历史子进程 exit 2 根因；各端真实重启消费同一终态 |
| 模型正文流 | 最终 canonical 回答真实通过；工具回答不冒充正文流 | 真实正文 delta→Store→客户端的完整正例与首输出样本 |
| 上下文维护 | 见上下文模块专项历史回归 | 热分叉完整覆盖、真实截断续写、多工具端到端结果 |
| 请求性能 | 本地 SQLite 基线、单次 provider sampling | 发送前全请求清单、canonical receipt 对账、失败/取消分母 |
| usage 与 UI | 正式 usage 账本及诊断接口存在 | 会话累计缓存率实测、实际可见帧与滚动/切换延迟基线 |

Task 预算恢复缺口修复（2026-10-12，HCTRL-06B）：`run_shell_task` 与 `run_agent_task`
已移除 `.ok().flatten()` 的预算降级，共享严格执行准入。读错、缺 Task、缺 scheduling、
损坏结构均拒绝；默认仅在创建时冻结，恢复不能补造。Shell 复用原 Task 的取消状态并在
拒绝时释放句柄；Agent 保留已封存结果的幂等 reconciliation 优先级，新执行前才要求预算。
`RuntimeTask::scheduling` 不再把非对象 metadata 当作未配置。

统一定向验证：Application Task 21/21；CLI Task 83 通过、1 忽略、0 失败，含两次冷恢复
子进程通过；exit 0。日志 `/tmp/nola-final-verification.gMkJRJ/task-scheduling-admission-2.log`。
真实隔离 Store 的 Shell 入口反例覆盖缺 policy、非对象和损坏 scheduler，断言哨兵/输出/
status 文件未创建，原任务及事件不变，句柄释放。Agent 尚缺损坏预算下真实模型请求计数
反例在当批尚未补充，不能将共享 helper 和代码位置当成该端完整验收；历史冷恢复 exit 2 根因亦未关闭。
首轮 `task-scheduling-admission-1.log` 编译失败，因累计 output_bytes 残留 Option 读取；
已改为直接读取持久累计值后整批复验，不把首轮计为通过。

Agent 预算入口补验（2026-10-12）：新增真实隔离 Store＋`run_agent_task` 反例，从已绑定
Run 与 pending Host effect 开始，对缺 scheduling、null、数组、损坏预算四类输入验证
原 Task/事件不变、Run 数不变、effect 身份/状态/摘要不变、delivery attempt 数不变。
首轮 `agent-budget-recovery-1.log` 为 11 通过/1 失败：查出拒绝时 Agent 控制句柄被释放，
但独立进程内 lease registry 未释放；不影响持久任务，却违反本地资源生命周期。
现按精确注册句柄释放 lease（已有 Arc identity 比较保护后继 worker），再扩大到 CLI
Task 测试组复验，84 通过、1 忽略、0 失败，`agent-budget-recovery-2.log` exit 0。该测试不发真实 provider 请求，
也不是 kill 进程验收；它补齐生产 worker 入口对已有模型工作不得派发的持久边界证据。

后文按时间保留原始失败与复验记录；“当时待测”不是当前状态，新通过也不能自动关闭
上表不同范围的门。性能采样、回答正确、工具副作用、Run seal、UI 展示分别验收。

- CLI 第六轮完整库 **1,732 通过、0 失败、4 忽略**，exit 0，215.32 秒；日志
  `harness-cli-lib-tests-6.log`。合法 genesis 元数据与空正文并存的 encrypted HTTP
  回归已通过。冷恢复重复子进程在第五/六轮均通过，但第四轮未知 exit 2 尚未取得
  原因，仍保留诊断追踪，不能声明性能改善（本轮与上轮负载不同）。
- iOS 收尾第七轮为定向 9 suite：**63/63 通过，零跳过、零失败，exit 0**，
  日志无 warning，`Results-7.xcresult` 已 finalized。覆盖固定取消命令/新派发时钟、
  实际模型开始后的原 pending envelope 冷恢复、权限失败立即重试、全局模型选择及
  第六轮原四个失败。不是再次运行全部 618 项。产物
  `/tmp/nola-ios-final.zmOU0Z/{tests-7.log,Results-7.xcresult}`。
  XCTest guard 已释放，Mac 真实 helper 六项（mock provider）组合随后执行。
- 收尾定向批：Core Host 第六轮 **142/142**，Service 第四轮四目标合计 **6/6**，
  均 exit 0，无忽略。日志 `host-contract-6.log`、`goal-task-service-4.log`。
  原取消 typed boundary 的首次准入与重启后字节不变性、Goal 归档崩溃补发布、
  worker 恢复/取消及 guidance 原正文快照均通过。CLI 第六轮与 iOS 第七轮定向
  复验仍在执行，此条不代替其结果。
- Core Host 第五轮 142/142 通过，`harness-host-contract-5.log`。
- Service 第三轮 5/6 通过，`goal-task-service-3.log`：Goal 已发布归档冷读与持锁
  崩溃后的补发、worker 恢复/取消、guidance 原正文固定窗口均通过。唯一失败为
  `agent_command_cold_restart_e2e` 用原始 submission JSON 比较规范化 boundary envelope；
  日志证明 cancellation_id/reason 不变，仍须保留规范化后的首租约字节比较并复验。
- Rust 库第五轮首个命令在测试启动前中止（exit 130），改为显式隔离 NOLA_HOME/
  NOLA_DB_PATH、16 MiB 测试线程栈和两个 workers；实际结果以
  `harness-lib-tests-5-isolated.log` 为准，不将中止命令算通过。
- Mac contracts 第五轮执行中，iOS 第六轮等待其释放 XCTest guard；本批不再穿插
  源码变更，先完整收集失败。日志分别沿用各自隔离临时目录。

后续已结束的 Rust 第五轮：`harness-lib-tests-5-isolated.log` exit 101。Core 472/472、
Store 403 通过/1 忽略（含 genesis 严格准入反例）；CLI 1,731 通过/1 失败/4 忽略，
失败为 encrypted HTTP journal fixture 仍断言 admission window 无任何 head，新契约
应有 exact sequence=0 初始 head 而无正文。待本批结果收齐后集中修正两项 fixture。
原 CLI 冷终局恢复两次子进程均通过，但第四轮 exit 2 原因仍未解释，新增诊断不等于
根因修复，不能以本轮通过掩盖。Mac 第五轮完整通过后释放 guard，iOS 第六轮已启动。

## 2026-10-12 最后统一验收（进行中）

最新 CLI 全库首轮：`cli-lib-tests-1.log` exit 101，1,712 通过、17 失败、4 忽略，
执行 212.99 秒。当前按完整失败集批量整改，未复验。含两份千万字符会话恢复测试，
长时间无输出期间已核实测试进程仍有 CPU 活动，没有重启或重复启动。
链接器另外报告测试可执行文件 `__eh_frame` 超过 16 MiB 的 unwind 警告，尚未处理，
不以静默屏蔽冒充解决。Android 最新验证 104 suites／402 tests，0 failure/error，
2 skipped；编译 androidTest Kotlin，不代表真实设备测试。

首轮失败批次已完成源码修复与静态审查，启动 `harness-lib-tests-2.log`：
Core＋CLI 完整 `--lib --no-fail-fast`，不是只重跑失败断言。生产修复包括 delegated
Completed 验证后、effect 退休前保存原 decision source，以及巨大 typed 工具结果
真正投影为持久化引用＋预览（不只缩短 display）。新增 invalid delegated payload
强类型来源不变性断言，避免读错 JSON 路径而两边 null 假通过。Mac contracts-2 与
iOS 第三轮按全局 XCTest guard 串行安排，未返回结果前不能标通过。

iOS 第三轮 `tests-3.log` 及 Mac 第二轮 `contracts-2.log` 均 exit 65、测试执行 0：
旧自动化展示仍使用 completed 或未穷尽 canonical result 状态，已分端批量修复源码。
Core＋CLI 第二轮 `harness-lib-tests-2.log` exit 101：Core 472 通过；CLI 1,727 通过、
4 失败、4 忽略，执行 158.14 秒。三项失败是旧导航文案断言，改测 typed 引用并保留
真实原文读回；另一项把 CLI 本地队列当远程提交，改为原客户端不得接管本地 Run
的安全反例，合法远程正例使用真实 Web register_stream_submission。未放宽授权。
另外 `harness-host-contract-2.log` 全 integration 120 通过、21 失败，新增委托来源
测试先卡未使用冻结 turn identity 的恢复配置；其余分为 requirements 控制工具投影、
旧隐式强制验证/完成语义及压缩取消晚帧。正在成批修复，未以删测试或恢复旧策略收口。

本批实现及静态审计收口后开始执行，不沿用下文历史通过记录。
第三轮 Rust 原始日志 `harness-lib-tests-3.log`：Core 472 通过，CLI 1,730 通过、
1 失败、4 忽略（148.07 秒），exit 101。唯一失败是新安全反例比较 raw 与 typed JSON，
两个 memory_policy.minimum_confidence 的 f32 往返差异；原文对比修复待复验。
`harness-host-contract-3.log`：138 通过、4 失败、0 忽略（2.38 秒）；委托终局来源与
非法 payload 不造来源的回归已通过，仍不能称全套通过。
Mac `contracts-3.log`：exit 65，但已执行完整 XCTest，989 tests／2 skips／170 failures
（103 unexpected），176.920 秒。大量失败指向共享 fixture model capability 必填字段，
另有独立 UI／签名／本地化问题，正在按根因分组；不是 170 个独立缺陷，也不是允许忽略
的测试噪声。iOS 第四轮日志 `tests-4.log`、`Results-4.xcresult`：exit 65，编译失败，
0 测试执行；HomeComponents 第九处状态 switch 和 RecurrenceRule actor 隔离已修，
连同运行记录 canonical 状态分类/计数源码一起待下一轮完整复验。
第四轮 Host contract 已完成：`harness-host-contract-4.log` exit 0，142 passed、0 failed、
0 ignored（1.12 秒）；含取消晚到摘要、委托来源与无效 payload 不污染来源、显式需求
证据及未完成收束。Rust 库及 Mac contracts 第四轮仍在运行。新增外部工具原文保存失败回归使用独立
临时文件阻止目录创建，不靠 chmod／真实数据目录；断言不可读引用、不提供续读偏移、
来源验证要求保持、未知副作用恢复提示和非空有界预览。未返回前不记录为通过。

第四轮完整库最终结果 `harness-lib-tests-4.log`：exit 101；Core 472 passed，CLI
1,731 passed／1 failed／4 ignored（138.43 秒）。新增输出保存失败回归及原 checkpoint
字节不变性测试已过；失败为 `bound_terminal_worker_recovery_never_restarts_model`，
独立子进程返回 2，尚待阶段诊断，不认定为偶发噪声。`goal-task-application-2.log`
四目标 24/24 通过；`goal-task-service-2.log` 四目标 3 passed／3 failed，真实 worker
崩溃/取消恢复三项过，另外三个失败根因分组与下一批修复见 AUDIT，未称全链路通过。

iOS 第五轮 `tests-5.log` 已编译并实际跑模拟器，但收尾两分钟无新增，单独对本轮
测试 xcodebuild 发 INT。当前原始日志有 620 条 case 结果（590 pass／21 fail／9 skip），
不是最终完整套件计数；xcresult 尚未 finalize，不能宣称完整验收。失败诊断保留在
`Results-5.xcresult/Staging/1_Test/Diagnostics`。已证实含缺 model capability fixture、
跨 Run journal owner、Host attempt 时序、receipt schema 等不同组；两项新运行记录
状态分类测试通过。另有三处源码 actor 隔离警告待修。禁止因测试中断删除原始证据或
把未完成项记成通过，下一批集中修复后重新完整验证。
Rust 使用仓库 cache wrapper（实际选中 mbx），隔离
`/tmp/nola-final-verification.gMkJRJ/home` 与 `acceptance.sqlite`，测试栈配置 16 MiB。
首次 `nola_cargo check --workspace --all-targets --quiet` exit 101：
Domain `assistant_journal_operations.rs` 的 ActivityState 二进制编码遗漏 Sealed，
尚未进入 Rust 测试执行。修复及复验进行中，原始日志为同目录 `workspace-check.log`。
后续复验：第 2 轮发现 Store port 权限 Debug impl 缺闭合括号，已补；第 3 轮
停在 Core requirements 工具 schema 的 `json!` 宏递归上限，正在拆分子 schema，
不以提高全 crate 限制替代源结构修复。日志 `workspace-check-2.log`、
`workspace-check-3.log` 均 exit 101。Sealed 已有独立 tag 7 和往返测试源码，
不能因为越过其编译错误便声称对应测试通过。mbx 报旧预测中的已删除 ffi.rs 未命中，
属于缓存预测警告，不恢复已删除接口。
第 4–6 轮继续暴露当前工作树集成错误，均 exit 101：Core 强类型 ID 转换与旧权限
方法、Store 测试 JournalNumber 路径已修；第 6 轮停在 Application 的 HostEventInput
归属字段、新增要求事件 match 及测试导入等接线。修正保持原身份校验，不用 wildcard
吞事件或添加旧 API 兼容。原始日志 `workspace-check-4.log` 至 `workspace-check-6.log`
保留；目前仍没有全工作区编译通过证据。
第 7 轮采用 `--keep-going` 暴露测试组织与旧接口残留：requirements 子测试移入
`tests/support/`，事件断言迁移至 envelope/Part，压缩计划断言通过 ID 与 fingerprint
从原 context 读取 retained items。共享 Store `settle_fixture` 实际也被 src 单测引用，
已恢复原 helper（仅对不用它的 integration include 标记局部 dead_code），不删断言。
单独 production check 发现 CLI RuntimeLlmProvider Debug impl 缺括号，已补。
当前正运行第 8 轮完整 all-targets，日志 `workspace-check-8.log`；第 7 轮及
`production-check-1.log` 均为失败记录，不能当通过证据。
共享模块 `nola_cargo check --workspace --exclude nola_cli --all-targets --keep-going --quiet`
已 exit 0，日志 `shared-check-1.log`；这不包括 CLI/原生 Host 产品闭包。
随后启动 Domain、Kernel、Core、Store、Application、Runtime Driver 六个 package 的
`--lib --no-fail-fast` 测试，日志 `shared-lib-tests-1.log`，exit 101：
Core 472、Domain 147、Kernel 124、Runtime Driver 2 项全部通过；Application
306 通过／4 失败／1 忽略，Store 379 通过／23 失败／1 忽略。失败集中于精确
assistant 准入、sealed predecessor／terminal commit 证据及不可变 input 反例。
当前逐项区分旧夹具与生产缺陷，不能删断言或放宽不变量；修复后须复验。
Web/测量脚本及 Android 由独立验收任务执行，未返回结果的项目不视为通过。
原生闭包 `nola_cargo check -p nola_cli --no-default-features --features native-host
--bin nola-agent-host --quiet` exit 0，日志 `native-host-check-1.log`。mbx 同时报告
catalog.rs 在编译期间变化而未保存缓存，因此冻结后仍需复验，不能以此作为最终
不可变源码快照验收，也未证明 Mac 打包／实际启动已通过。
Runtime Driver 两个 integration target 已执行，日志 `harness-driver-e2e-1.log`，
exit 101：`opaque_protocol` 7/7 通过；`committed_runtime_e2e` 7 通过／5 失败。
失败包括静默／连续帧下短租约过期、两个精确心跳计数，以及末尾排队后继输入未出现在
预期会话投影。当前修复阶段保留失败记录，不把两个库测试通过视为完整 Driver 验收。
Driver 第 2 轮 `harness-driver-e2e-2.log` exit 0：committed runtime 12/12、opaque
协议 7/7。短租约夹具改用注入 epoch 时钟，生产默认时钟和 Instant heartbeat cadence
保持不变；续租按已接受 Started／最新 ACK 证据验证。NextTurn 不再期待旧 Run 自动
吞并后继，而验证旧 Run seal、冷重开、正式新 Run 执行和完整消息序列。此处为真实
Store＋fixture Host 集成通过，仍不是实际 provider／GUI 验收。
Application／Store 复验第 2 轮在新增 seal 断言缺 trait import 处编译失败（已补），
日志 `store-application-lib-tests-2.log`。第 3 轮同名 `-3.log` 中 Application
已 310 通过／0 失败／1 忽略；Store 389 通过／13 失败／1 忽略，未算通过。
本批 Automation occurrence 已补持久 definition_revision；Mac／iOS 模型、调度入队及
lease 使用已提交版本，补缺字段拒绝与冷重开版本断言。六个相关 Swift 文件 frontend
parse exit 0（`automation-swift-parse.log`），仅语法检查，不等于类型检查／XCTest 通过。
Automation Store 四个 integration target（automation_e2e、automation_delete_e2e、
automation_revision_e2e、automation_run_e2e）共 9/9 通过，exit 0，日志
`automation-store-e2e-1.log`，包含定义变更后的原 occurrence revision fence 反例。
Application 纵向三 target 第 1 轮 `harness-application-e2e-1.log`：权限决定
10/10 通过，取消 10 通过／1 失败，task/automation 绑定 14 通过／1 失败。
取消测试仍把 Run 生命周期写成 cancelled，已改验证 sealed ＋真实 Cancelled 结果；
任务 reconciliation 反例手写 sealed 状态缺 seal，已改先通过真实 Host 结果封存。
两处测试源码已修，待复验，不能把这一轮记成通过。
上述三 target 第 2 轮 `harness-application-e2e-2.log` exit 0：权限决定 10/10、
取消 11/11、task/automation 绑定 15/15，共 36 项通过；保持真实封存与取消业务结果
分离的断言，未以放宽 Store 证据要求换通过。
Store／Application 库测试第 4 轮 `store-application-lib-tests-4.log`：Application
310 通过／1 忽略；Store 400 通过／2 失败／1 忽略。剩余是远端 seal 导入夹具
缺失读取证据，以及取消变体的 exact session/revision replay 身份，继续修复中。
完整 Rust 生产目标 `nola_cargo check --workspace --lib --bins --keep-going --quiet`
exit 0，日志 `workspace-production-check-2.log`，本次包括 CLI；不含 test targets，
也不替代 native-host feature 独立闭包、Swift／Kotlin 或真实运行验收。
Application／Store 第 5 轮 `store-application-lib-tests-5.log` exit 0：Application
310 通过／1 忽略，Store 402 通过／1 忽略，无失败。最后两项分别补齐远端 message
admission frontier 和取消提交的原 Core session/revision 证据，未放宽生产准入。
完整 Rust `--workspace --all-targets --keep-going` 第 10 轮 exit 0，日志
`workspace-check-10.log`。第 9 轮只剩 CLI 恢复夹具把 HostRuntimeOptions 整体赋给
RuntimeConfig 的类型错误；已改显式同步该夹具改变的三项配置，没有恢复旧类型别名。
这证明所有目标可类型检查，不等于所有目标测试已执行。

## 当前验证策略与状态

按用户最新要求，整个 goal 的实现、文档、测试源码与静态审计先全部完成，最后集中
编译、运行测试和真实端到端验收。现已进入该最后阶段，最新结果见本页顶部及各端
当次验收记录。下文历史通过记录仅证明对应旧代码状态，不能覆盖本批结果契约、
V2 seal、恢复、Task 和跨端消费者；标记“未运行”的旧源码记录不代表后续已通过。

当前静态发现并整改：Application 终态 Run 写入改为 sealed，消息状态仍独立；Kernel
普通正文不再因待办布尔值自动重试；checkpoint 终态不能由启动恢复补写 Run 终态；
取消提交补齐原命令身份。待完成入口清理、跨端协议和完整纵向审计后统一验证。

### 2026-10-12 控制与恢复批次：统一验收入口

本表是最终统一验收的待执行清单，不引用下文旧通过数替本批背书。

| 契约 | 源码入口／反例 | 必须补齐的纵向证据 |
|---|---|---|
| HCTRL-01 静默控制输入 | Runtime Driver `tests/opaque_protocol.rs`：首帧前静默仍准入、成功后 ACK | 真实 CLI/Mac provider 无输出时提交补充与取消，核对 durable input、Core 接受、UI 可见反馈；分别测控制准入和空轮询开销 |
| HCTRL-02 控制失败停止作用域 | 同一文件：保留未 ACK 输入、拒绝迟到帧；多 effect 的有帧/无帧失败须一致 | source/admission/ACK 三处故障下不创建后继副作用；跨进程恢复保持原输入 ID；协作 abort 不能当进程已终止证据 |
| HCTRL-03 局部失败隔离 | `effect_failure_preserves_sibling_control_admission_and_later_scope_failure`：局部 effect 失败后，静默/Started/Delta 三条路径仍处理公共输入，随后真正 scope 失败才关闭全部 | 两个真实 Host 工作并行，其中一个失去 fence，另一个静默；确认取消/补充仍进入原 Run，失败 effect 不重复 abort、不提交迟到帧；源码九种组合不替代实际 Host 取消及阻塞回调测量 |
| HCTRL-04 权限决定原子性 | `permission_request_port_e2e.rs`：相同决定重放、冲突、缺失以及 grant/sibling/原决定同事务 | CLI/Mac 确认后丢 ACK、撤权后重放、两个客户端相反决定并发；只认 canonical receipt，不复活 grant、不由界面补写授权 |
| HCTRL-05 取消与后台转移 | `tools/parts/background.rs`：取消优先、取消提交只接受明确 ACK；默认控制句柄不能自授后台能力 | 真实长任务上并发取消/手动转移/自动转移，检查原 task/attempt 未换身份；取消 ACK 后须有停止或未知证据，不能仅观察 UI 按钮消失 |
| HCTRL-06 Shell 提交不确定 | `task_runtime_parts/execution.rs`：save 后 launcher 错误仍保留记录；`agent_host_driver_parts/orchestration.rs`：五个失败阶段保留 Unknown receipt | submit/save/claim/worker ACK/registry 各窗口故障后重启，关联原 task/attempt 和 stdout/stderr/status，检查副作用计数；查询与等待遇 queued/授权等待/存储失败不得结算完成；stop 不得以发出信号代替停止证据 |
| 协议修正终局来源 | Core `runtime/requirements.rs`：空/陈旧 Model 来源；Store `run_requirements.rs`：协议 Error 事实绑定及 seal | 真实协议拒绝后修正未完成的收尾，核对同次事实→Incomplete→seal→客户端显示；不得造 Blocker，不得借旧 model_completed，也不得因落库失败隐式重跑工具 |
| 原生启动边界 | `native_host_launch.rs`：显式 owner、冷取消不调用 cwd、拒绝重复权限参数；`MacReasoningConfigurationTests`：权限只走 stdin，本地凭据不进 argv | 当前构建 Mac 从提交到 stdin、Core 及落库，三种权限模式与取消/恢复逐项核对；失效工作目录不阻塞显式路径取消；Server/session/worker 功能仍可用；依赖闭包需编译与架构门禁证明，不靠新文件名 |
| 错误展示不结束执行 | iOS `ChatErrorPresentationTests`、Android `ChatErrorDismissalTest`：旧会话/旧 occurrence 不清新错误，不清执行内容 | 两端实际执行中断→关闭提示→切会话→收到同文新错误→恢复/取消；核对原 Run、部分正文/思考/工具事件保留，新会话 composer 不继承旧运行状态；纯展示 helper 测试不能代替订阅接线 |
| Goal 调度不得隐式恢复 | Core 恢复反例与 `goal_idle_cold_restart_e2e`：真实 service 重启后领取 Suspended 被拒，独立读取 Goal/revision 不变，再显式 Resume | 当前只补测试源码；最终运行真实 service fixture，并从 CLI/Mac 暂停入口到调度器领取验证。等待用户、失败、Incomplete 不得被 worker 的 Claim 改成 Running；租约不是恢复授权 |
| CLIENT-01R Mac 决定持久化 | `MacUnknownEffectRecoveryControlTests`：丢回复、原载荷重放、局部 rejected、超限记录、跨 Run 隔离 | 真实 Application 提交后断连接、客户端重启，读取原 receipt；原 resolution 只记一次，风险重试只恢复原 Run |
| CLIENT-01U Mac owner | `MacUnknownEffectRecoveryPresentationTests` 与 `MacLocalRunTerminalContractTests`：冻结 owner、切会话旧点击 | 原生界面确认/取消、会话切换、冷启动；Blocked 展示必须对应原 Run canonical seal，不依赖脚本化 reply |
| A19 iOS 回执与 journal | `IOSUnknownEffectRecoveryTests`、`IOSPendingCommandJournalTests`：完整 envelope、实际序列化上限、坏文件拒绝、跨实例事务互斥 | 外层 FFI 错误、坏终局 payload、持久写失败、重启均保留原意图；用户入口到原 Run 恢复及 seal 全链路；查询可继续而新写入受原意图约束 |
| CLIENT-01R Android 决定持久化 | `ApplicationSubmissionJournalTest`：旧实例重载、事务互斥、坏回执和业务结果保留；`AndroidSubmissionRecoveryNativeTest`：真实 JNI 冷重开与 pending 期间只读查询 | 设备实际目录权限、文件及目录同步、进程中断后 receipt-first；Compose 确认到原 Run dispatch/seal；现有 JNI pending 夹具不含真实未知副作用，不能替代该链路 |
| CLIENT-01R Web 浏览器决定 | `unknownEffectRecovery.test.ts`：冻结 owner/决定、receipt-first、丢 ACK、跨 Run 隔离；服务端 `unknown_effect_recovery.rs` 的原 admission 归属与同 ID 改载荷拒绝反例 | 真实 IndexedDB 写失败与浏览器重启、会话/CSRF、原 client 绑定；同 owner/request ID 改 reason/time/fence 必须冲突而非新执行，且冲突不得清除原决定；身份哈希已移除载荷，仍待运行验收；当前 Application fixture 仅覆盖 missing-attempt conflict，不能证明成功风险重试 |
| A18 单一执行 ABI | `workspace_dependency_architecture` 原生裸 Core 门禁 | 编译后的 header/库/JNI/Swift/Kotlin 消费者一致；无裸可变 Core 导出，Application ABI 与命令协议不符时明确拒绝 |

每个纵向样本记录原 command/request、conversation/Run/assistant、attempt/lease revision、
receipt 和 seal 的关联（公开报告不含 lease token），并列出注入故障的位置及原副作用计数。
只有 UI/协议 fixture 的项目仍标记未验收；旧二进制启动成功不能证明当前工作树通过。

### 实现冻结与统一验收顺序

实现冻结不是整体完成。进入统一构建前，须静态核对以下跨模块链，不用单个 agent 的
“已完成”替代调用方整合；发现真实契约矛盾可继续整改，不先运行零散测试背书。

1. 控制输入：Host 注入不经过键盘/composer；首次发送前冻结意图，模糊 ACK 保留原身份；
   同文本的新提交与恢复原提交是不同 typed 操作。重启恢复的客户端意图文件不是业务 FIFO，
   原 received-command receipt 与 Application 输入队列仍为唯一提交/消费依据。
2. 权限决定：首次决定、相同决定重放、相反决定冲突、请求不存在分别返回；授权 grant 与
   首次决定原子提交。唤醒已有等待不得由 CLI 根据旧 scope 快照批量批准；回执重放不能
   复活已撤权 grant。字段、生产消费者与反例源码须同批切换。
3. Goal 接受：逐要求 policy 与版本绑定真实来源；模型判断不冒充执行检查，子失败可用
   显式替代证据处理。拒绝完成不能无条件生成下一轮，也不能一概转为等待用户。
4. 单版本契约：本批 permission result 与 Goal 必填字段全部定版后，再统一更新协议/
   checkpoint 版本和原生消费者。核对 Protocol 常量、Mac service client、Android ABI/协议
   探测、Swift FFI 探测以及 protocol fixtures；不得用缺字段默认值维持旧协议。
   本批源码已将 Application service protocol 统一为 49：权限决定结果为 tagged receipt，
   Mac/iOS/Android 显式检查同一版本，拒绝前后版本的反例同步；Goal checkpoint 为 16。
   这仅是源码切换，未证明已链接库或运行中的客户端更新。不匹配的旧 pending 保留并报错，
   不转换原请求、不更换 ID 重发，也不自动删除 journal/数据库。
5. 模块编译闭包：A05 输入服务拆分只是第一条反向依赖。Config、Emitter 与入口装配的
   CLI/Web 引用仍须收敛，不能把同 crate 新文件称为 Host 独立编译边界。
   当前 native Host 已独立入口并将终端依赖移入可选 `terminal-ui`；Mac 脚本选择
   `--no-default-features --features nola_cli/native-host`。新增 manifest 门禁源码核对
   三个终端依赖为 optional、CLI target 必需 terminal-ui、Host target 不要求它。
   此门禁只证明声明形状；最终必须实际运行
   `cargo check -p nola_cli --no-default-features --features native-host --bin nola-agent-host`
   并验证 Mac 打包产物，才能证明生产模块没有引用被排除的终端代码。

冻结后一次进入构建/测试阶段：共享协议及 Rust 生产闭包 → 各端编译与契约/故障测试 →
隔离数据库上的真实客户端纵向操作 → 固定产物性能基线。各阶段失败应修正后重验受影响
链路；“最后统一测试”不意味着发现失败也只运行一次。最终把原始命令、产物摘要、环境、
故障位置、完整分母和 canonical 最终证据关联存档，再逐行核对下文整体完成判定。
尚无真实基线的性能项保持未定预算，不以轮询间隔、fixture 耗时或 adapter 首输出代替 UI SLA。

历史正文范围恢复夹具已迁移到当前 Core 来源事件的 Store 边界提交，并将 journal、V2 seal
置于同一外层事务。inline／归档正文／stream 的原冷恢复和跨归属拒绝断言保留；消息
completed/failed/cancelled 仅决定正文读取资格，不作为 Run 成功证据。此夹具不是实际
模型或 Core reducer 端到端测试，尚未运行；不得用于宣称语义结果已经真实验收。

Web 会话缓存统计静态审计发现概览仍读取最近请求而非会话累计值。新增
`GET /api/chat/conversations/:id/context-usage`，沿现有 Web 会话认证边界调用
`LoadConversationContextUsageV1`；返回明确 conversation_id 与 Store 请求账本累计投影，
不在 Host/浏览器累计事件。前端解析器与未知用量、不同会话、加权统计反例源码已补。
概览组件已接会话累计数据，七语言标签明确累计范围并列出已上报／未上报请求数；
未上报不显示 0 命中率。切换会话主动取消旧读取且以 generation 隔离迟到响应，
并发刷新单飞合并，只在会话选择、已提交 usage 前沿、终态和返回页面时刷新，不逐 token 查询。
新增读取失败、切换、同会话重选、合并和卸载测试源码，均未运行；真实 UI 验收仍待完成，
不能记为已通过。iOS 观察测试的 context record 已改为
V2，请求序号独立于上下文事件序号；权限拒绝后的正文只断言 Answered，不断言动作成功。

客户端协议静态复核另补 Mac/Web/Android 的旧 context record fixture，以及 Android
终态读取、观察 fixture 缺失的 V2 完整 result/seal 字段。Web child receipt 结构检查
增加 child session/run、sealed 状态与 Task 唯一性，附旧状态、缺身份／摘要、重复 Task
反例源码；消息 completed 仍是独立消息闭合状态。以上均未编译或运行验证。

三端 Task sealed 投影已保留完整结果/摘要/receipt，原生与 Android 结构检查按 Domain
约束拒绝缺失依据、互斥 effect、坏 reason、越界计数和空前缀矛盾；不是本地证据验真。
旧 Mac 独立 reducer harness 改用真实 App XCTest。Web Journal 只读状态标签覆盖合法
Task 等待、暂停、重试、封存和取消态，七语言文案及纯映射反例源码已补；sealed 不显示
业务成功，未知态不直出协议字符串。上述新增源码均待最终统一测试。

## 当前声明与权限准入批次（2026-10-11）

### 尚未运行的新增门禁

- Web `runResult.test.ts` 覆盖六类业务结果、空阻碍证据、无剩余事项的 Incomplete、
  observed/unknown 效果交集、身份控制字符、稳定 reason code、i64 数值边界和
  空输入水位／摘要承诺矛盾。解析器只验证结构，不赋予引用来源真实性。
  Mac/iOS/Android 对应结构校验正在同步；不能以 Web 覆盖代替原生端验收。
- HTTP 发送工作线程断开不再返回用户取消。传输枚举新增 `WorkerDisconnected`，
  Chat/Responses/Anthropic 消费者按请求故障处理；只有显式取消上下文才返回 Cancelled。
  新增断开无取消、断开且已有明确取消两项源码反例。未运行，不代表真实网络验收完成。
- Web `canonical_web_host_e2e` 源码覆盖全部十个页面 GET/HEAD 使用同一 Svelte shell；
  fixture 保留旧模板而移除 Svelte 入口时必须返回 503。Rust 旧模板渲染链已删除，
  但旧静态文件及其引用测试仍需审计；此测试不替代浏览器交互验收。
- Measurement 首次文件打开、JSON 序列化、刷盘均移出业务线程。增加写入失败使
  诊断接收端断开的反例源码；`enabled()` 不能证明观测已经写入文件。
- Repository 写入守卫记录诊断暂存前沿；显式回滚和 Drop 回滚都截断本层新增记录，
  保留外层尚未提交的观测，防止随后外层 COMMIT 把已回滚效果记为成功。补两种路径
  的嵌套反例源码，尚未运行；性能记录不替代 Store 原始证据。
- 依赖门禁允许 Store 使用 Measurement，但禁止 Measurement 反向依赖 Store、
  Domain 引入 Measurement。新的 manifest 白名单和反例尚未运行。

本批删除缺 descriptor 借用旧 catalog、空模型 profile 自动补能力、权限旧拼写及坏配置
静默回退。所有成功测试 Host 改为显式声明；无效输入反例仍保留。未删除用户数据库。

- Core lib：456 passed。
- Core `agent_core_host_contract`：130 passed；`agent_turn_context_snapshot_contract`：2 passed；
  `model_capability_contract`：6 passed。涵盖本次缺 descriptor 拒绝、完整 checkpoint 不变、
  同响应序号修正后成功激活；不代表真实模型的能力发现链路验收。
- Application lib：306 passed、1 ignored（子进程测试入口），9.57s。
- Android `AndroidModelCapabilityProfileTest`、`AndroidTurnModelSnapshotTest`：各 3 passed；
  Gradle `testDebugUnitTest` 成功，非设备端执行测试。
- Mac `Test` 配置定向两项：坏权限元数据读取不改原设置、三模式 round-trip/default；
  XCTest result summary 确认 2 passed、0 failed。未运行整套 UI 或真实 Provider 测试。
  产物：`Test-NolaMac-2026.10.11_8-44-48-+0800.xcresult`。

首次统一验证暴露一个纯文本模型 fixture 缺 revision；补齐后上述三组 Core 集成全部通过。
工作区检查曾发现 CLI footer 旧函数参数和跨 crate fixture 的父模块依赖；均已修正。
Rust `check --workspace --all-targets --quiet` 重验 exit 0；CLI `--bin nola permission`
筛选 110 passed、0 failed，24.28s，包含 `/allow` 切换与持久权限等待回归。
iOS 首次选用 Test 配置被 Xcode 拒绝（测试目标不支持该模拟器平台），未执行测试；
改用独立 `build` 目录的 Debug-iphonesimulator 配置后，权限模式与坏存储配置定向 5 项全部通过；
已读取 `/tmp/nola-ios-contracts.zU1SYp/Permission.xcresult` summary 验证 5 passed、0 failed。
Application `--tests --no-fail-fast --quiet` 已结束（exit 101），共 10 个目标失败，
不能以此前 lib 306 项通过覆盖当前反例：

- `assistant_turn_binding_production_e2e`、`model_completion_kernel_receipt_e2e`、
  `run_recovery_checkpoint_identity_e2e`、`successor_turn_permission_resume_identity_e2e`，
  以及 `turn_transaction_e2e` 的三个后继测试仍要求同 Run 自动后继；应经生产显式准入
  建立独立 Run，并保留精确归属、恢复及权限断言。
- `--lib` 的只读 reasoning 查询遇到归档后台提交导致 `data_version` 变化；查询分支本身
  未写 Store。相邻只读测试也需隔离并发写入，保留不写入断言。
- `context_usage_read_e2e` 暴露真实协议缺口：远端只有最新请求用量，没有每 Run 累计统计，
  因而接收端丢失会话历史缓存命中率。需独立请求前沿、幂等替换、跨 Run 去重汇总。
- `nonterminal_replay_e2e` 仍构造 schema 55 并期待自动迁移，与当前单版本契约冲突。
  已改为当前 schema 中单 Run replay 损坏的隔离反例，尚未重验。
- `task_run_binding_e2e` 的删除遇到外键拒绝；不可变输入／证据索引与会话删除的生命周期
  契约尚未统一，不允许通过关闭外键解决。
- `work_archive_recovery_e2e` 缺归档文件；`turn_transaction_e2e` 的另一项回滚全表快照
  不相等，需核对后台发布与事务观察边界，尚未完成根因验证。

以上修复在统一重验前均不记为通过。
Mac 构建仍有 traditional headermap 配置警告，不能称无警告构建。
CLI 测试链接器提示 `__eh_frame` 大于 16 MiB，当前未解决，不能称无警告链接。
Rust 测试使用 mbx、独立临时 NOLA_HOME、16 MiB 测试线程栈；未证明默认栈或性能预算达标。

## 整体完成判定

### 当前修复批次复验（2026-10-11）

Store schema 67 当前实现：`nola_cargo test -p nola_store --lib --quiet` 已结束，
387 passed、0 failed、1 ignored，30.07s，exit 0。
包含单 Run 输入索引不可变及终态会话删除回归；不能推及跨三 Run 队列归档替换已验证。
日志 `/tmp/nola-contract-store-tests.log`。Application 全集成 `--no-fail-fast` 已结束，
exit 101；库测试 306 passed、1 ignored，7.83s。完整日志
`/tmp/nola-contract-application-tests.log`。剩余 3 个失败目标、4 项失败：

- `context_usage_read_e2e` 新多 Run 场景未先接收首 Run 终态，第二 Run 被活动保护拒绝；
  原有 9 项通过，已补真实终态传输步骤，尚待复验。
- `model_sync_atomicity_e2e` 参数重置报 database locked；发现独立多步模型写入使用裸
  savepoint 的延迟读转写风险。需统一显式写入准入，不能用等待掩盖快照升级竞争。
- `turn_transaction_e2e` 后继远端认证失败；已定位 successor 准入跳过 source receipt
  提交，属于生产事实链缺口，不得伪造 receiver root 绕过。
- 同目标回滚全表比较只有 `work_archive_publication` 变化（后台归档失败状态更新），
  业务表无差异；需明确测试观察边界，保留回滚断言。

本轮其余此前失败目标未再报错；上述修复及新增跨入口回归仍须统一重验。
第二轮已启动，日志使用 `*-tests-2.log` 保留上一轮证据。Store lib 389 passed、
1 failed、1 ignored（30.73s）：新增写锁准入、后继 source receipt 和同步活动 Run 保护
未失败；`unsupported_nonzero_versions_are_rejected_without_changing_stored_facts` 的
全库快照被其先前启动的归档线程改变。日志中 publication target=3 不变、published
由 0 变 3；需用既有无后台发布测试构造隔离旧版本拒绝检查，而非放宽 schema 拒绝。
第二轮 Application 已结束 exit 0：85 个测试结果组，638 passed、0 failed、1 ignored
（含库 306 passed、1 ignored）；统计来自日志各组结果之和，不代表 85 个独立业务场景。
三 Run 原始来源→冷 promotion→归档提交／重放、source receipt 后继准入、远端累计缓存
终态接收、多步模型写事务及回滚场景已在该轮通过。
Store 旧版本拒绝 fixture 已改用既有无后台发布构造，保留 schema／全表不变断言；
整库第三轮已通过：390 passed、0 failed、1 ignored，43.87s，exit 0。
工作区 `check --workspace --all-targets --quiet` 同批 exit 0；日志分别为
`/tmp/nola-contract-store-tests-3.log`、`/tmp/nola-contract-workspace-check.log`。
这是当前事务、准入、缓存同步与删除生命周期修复的 Rust 验证；不证明 Swift/Kotlin
新缓存 DTO 已重新编译，也不证明尚未接线的业务 RunResult/V2 seal 已完成。
统一运行使用隔离临时 NOLA_HOME、mbx、16 MiB 测试线程栈。

下面按原始交付要求判定，而不是按已通过的测试数量判定。后文执行记录保留其运行时点，
不得用较早版本的绿灯覆盖当前实现。当前整体状态：**未完成**。

| 要求 | 当前权威契约 | 完成所需证据 | 当前缺口 |
|---|---|---|---|
| 模块状态与修改权唯一 | MODULES、DEPENDENCIES | 生产调用路径及旁路审计、依赖门禁 | Host 包含 CLI、宽适配接口及各端入口仍需收敛 |
| 类型化输入输出与错误 | MODULES、TERMINAL_PROTOCOL、WAIT_PROTOCOL | 每个生产边界的正反例及 wire 回归 | 原终局／启动 wire 缺陷已修，Mac 原 Run 冷恢复及封存精确读回已过；五端所有边界仍需逐项验收，不能沿用历史“仍丢失”作为当前已确证故障 |
| 正常、失败、取消、受阻、恢复 | LIFECYCLE、COMPLETION、RECOVERY_AND_AUTHORITY | E2E-01/02/05/07/09/13 的完整状态转换 | 未知效果出口、受阻封存、取消竞争尚未全链验证 |
| 原子持久化与幂等重放 | TERMINAL_PROTOCOL、WAIT_PROTOCOL | 同事务断点故障、进程 kill、重复提交及恢复证据 | 已有 SQLite/CAS 子集不足以证明全部崩溃边界 |
| 实测性能预算 | MEASUREMENT、PERFORMANCE_BASELINE | P01–P08 当前产物的基线、失败分母与预算决策 | 真实 Provider 文本功能验收已过，尚无完整性能基线；当前接线可选阶段采集，压缩、工具及 UI 时钟／分母仍须补齐 |
| 用户操作到最终落库 | 本页纵向矩阵、CLIENT_BOUNDARIES | 真实客户端交互、Host 执行与最终 canonical 结果对应 | 五端与真实模型整链路未全部通过 |

完成评审必须逐行核对源码与本页场景证据；类型存在、文档存在或某一组件测试通过，
都不能单独把任何一行标为完成。各条未完成不允许改写为更窄的交付目标。

## 原始执行证据与乱序恢复（2026-10-11）

使用 mbx、独立临时 NOLA_HOME、16 MiB 测试线程栈；未使用用户数据库。
下列记录是 Host→Core→Application→SQLite 来源链的验收，不是 RunResult/V2 seal 完成证据。

| 命令 | 最新结果 | 范围 |
|---|---|---|
| `nola_cargo test -p nola_domain -p nola_host_protocol --lib --quiet` | 132/132、27/27 passed | 单一 receipt 类型、无证据/未知版本、原协议校验 |
| `nola_cargo test -p nola_core --test agent_task_handle_contract execution_evidence_contract -- --nocapture` | 3/3 passed，0.03s | 裁剪前事实、恢复身份、负向检查与部分失败 |
| `nola_cargo test -p nola_application --lib execution_evidence_tests -- --nocapture` | 5/5 passed，0.93s | 真实 Core/Host/SQLite，输出裁剪、重开、重复回执、错 Host 拒绝、原事件/索引篡改拒绝、并行逆序完成后重开再刷新 |
| `nola_cargo test -p nola_store --lib --quiet` | 399 passed、0 failed、1 ignored，30.82s | schema 65 迁移、来源索引与原有 Store 回归；ignored 为独立磁盘性能实验 |
| `nola_cargo test -p nola_core --lib --quiet` | 453/453 passed，0.34s | Core 现有库测试，不等同所有集成目标 |
| `nola_cargo test -p nola_application --lib user_input_wait_tests --quiet` | 10 passed、0 failed、1 ignored，1.35s | 持久提问与恢复；ignored 为测试子进程入口 |
| `nola_cargo check --workspace --all-targets --quiet` | exit 0，无诊断输出 | Rust 全工作区所有目标类型检查，包含 CLI/服务/移动绑定；不是 Swift/Kotlin 或运行时验收 |

首次整库运行曾为 392 passed、6 failed、1 ignored：五个旧版本迁移夹具未清理/声明新表，
一个清理夹具缺少当前 Core 消费证明。修复精确夹具并补 pending/missing/stale witness 保留测试后，
重新运行整库通过；没有使用 `IF NOT EXISTS` 或放宽来源校验掩盖问题。
扩大普通业务 `contract` 场景时曾误用内部事件作为公共事件，编译失败；已改为生产恢复后模型上下文断言。
普通 `contract` 字段（字符串、数值、null、缺失）保留业务含义，不能授予 receipt 信用；
未知内部 `nola.tool_result.*` 版本仍拒绝。此条已在最新五项 Application 测试中执行。

本次重开是关闭/重新打开真实文件库，不是此新路径的进程 kill 测试；没有 Provider 请求或窗口交互。
不证明默认小栈、所有客户端行为或性能预算达标；启动清理按 ACK 行查询当前 witness 的耗时尚待测量。

## 要求评估来源一致性（2026-10-11）

隔离临时 NOLA_HOME、mbx、16 MiB 测试栈下，
`nola_cargo test -p nola_domain --lib --quiet` 最新 137 passed、0 failed，0.10s，exit 0。
新增 5 项回归覆盖：已提供的新否定/证据不足不能被旧成功掩盖、同 cursor 同要求多判断歧义、
同一事件信封不得有不同 Turn/revision、cursor 顺序中的 revision 不得倒退；
同时保留数组顺序无关、同 revision 多事件以及跨 Turn 的旧失败后新成功路径。
这只检查传入集合的结构一致性，不能发现遗漏的持久评估、证明来源存在或替代 V2 封存验收。
本轮未修改生产完成决策或新接入要求控制工具；接线规格见 REQUIREMENT_ADMISSION。

## 测量规则

事件字段、现有计时点的实际含义、跨进程时钟及样本完整性见 [MEASUREMENT](../../软件架构设计/模块/15-观测与验收/MEASUREMENT.md)。

先测基线，再确定数值预算；缺测量的指标不得写成已达标。所有 latency 使用单机 monotonic clock，
跨进程由 correlation/span 拼接；没有时钟可比性时分别报告各段，不能直接相减不同机器墙钟。
实验必须记录设备、构建 profile/产物 digest、模型/协议/上下文窗口/思考与工具配置、冷热启动、样本数。
生产数据只读、日志脱敏；用户会话不能被为了测试而续跑、删除或替换。

## PERF 指标、口径与预算状态

| ID | 指标与公式 | 必需维度 | 当前预算/基线状态 |
|---|---|---|---|
| P01 | submit→durable ACK | 冷/热服务、DB 大小、平台 | 已测本地 Application/SQLite 子段，缺 UI/IPC 全链；见 PERFORMANCE_BASELINE |
| P02 | 用户首可见反馈、首 reasoning、首正文分别计时 | 排队/配置/context/provider/发布/渲染分段 | 缺同口径基线；不把 spinner 当 token |
| P03 | effect 排队、授权等待、执行、回执提交分别计时 | 只读/写入/验证、本地/远端、输出尺寸 | 已测固定 Host 输入的本地回执子段，缺真实工具及等待分段 |
| P04 | 一次压缩从计划提交到 context commit | 热/冷/分片/续写，输入/输出 token | 有个案，见下；不具备 p95 SLO |
| P05 | cache read / 可比较 input 总量 | 协议/模型/会话/普通与维护请求 | 按报告计数加权；unknown 单列覆盖率，不能按 0 混入 |
| P06 | commit→projection→首帧；滚动帧间隔、长帧率 | 流速、工具数量、长历史、会话切换 | 缺 UI 实测；不能用 reducer 执行时间代替 |
| P07 | 重启→恢复可交互、重放事件/读取字节 | checkpoint/replay 长度、未决 effects | 缺实测；必须同时验证不重复副作用 |
| P08 | 工具结果/每轮 prompt 字节、正文写放大、RSS | transcript 大小、平台、工具类型 | 有历史报告不等于当前版本基线，需重测 |

暂定不可突破的正确性预算：重复副作用=0、跨会话状态污染=0、已 ACK 事实丢失=0、
未知结果误报成功=0。数值延迟预算当前未批准，不能把“待测”作为无限等待的运行策略；
每个 effect 仍须继承已准入的 timeout/cancellation，超时按 LC 处理。

采样计划：同机固定配置，对每种冷/热环境至少重复 30 次；公开每个样本、median/p95/max 和失败率，
小样本 p95 仅作为描述不作为稳定性证明。Provider 延迟另列，不对不同模型/不同窗口做严格 A/B 宣称。
基线建立后明确用户体验目标、可控模块预算、超限策略与环境限制，登记到本表再进入回归门禁。
不可仅设置一个宽松的总超时来声称性能有预算。

## 已观察证据（不是通用 SLO）

| 场景 | 条件 | 观察 | 限制 |
|---|---|---|---|
| 最新真实 Mac 失败 Run | 2026-10-11，Qwen Responses，既有长历史 | 4 次冷摘要 elapsed 合计 179,847ms；11:15:52 提交摘要；cache read 均 0 | `no_request_snapshot` 原因需细分；整轮最终收尾失败 |
| 热分叉专项 | `/tmp/nola-mac-real-memory.FFsd8P`，40k/16,384 输出 | 3/3 分叉，命中 96.6%–96.9% | 新的隔离会话；不能代表旧会话恢复 |
| 真实截断续写专项 | `/tmp/nola-mac-real-memory.CRZsHK`，thinking=none，单次输出注入 128 | 506 字节片段续写成功；续写 19,380/19,433 命中约 99.73%；6 轮完成 | 受控故障注入；不代表自然满额截断或实际 UI 验收 |

临时目录可能被系统清理；验收证据缺失时必须重新采样或保留脱敏机器报告，不能仅引用旧文字。
后续报告需有受控保存位置、代码/产物身份和场景指纹。此项尚未全部落实。

## 验收层次

### 当前版本初始化与长正文关闭（2026-10-11）

按项目未发布、只维护当前版本的要求，Store 已删除旧 schema 升级链、来源回填、旧 payload
转换及专属迁移测试。全新空库当前 DDL/application_id/user_version 同事务建立；非空未知库、
非当前版本明确拒绝，不自动重建/删库。当前格式重开不重建结构；拒绝测试比较全部表内容、
sqlite_master 与版本标记不变。源损坏、原子提交、重放及当前格式冷重开测试继续保留。

冻结后，经项目 mbx、隔离 NOLA_HOME、16 MiB 测试栈统一执行：

- Store lib：**386 passed / 0 failed / 1 ignored**，29.74s；ignored 是显式磁盘性能基准。
- Application lib：**306 passed / 0 failed / 1 ignored**，6.40s；ignored 是由父测试启动的进程入口。
- workspace all-targets check：exit 0。mbx 提示旧预测引用已删除的迁移文件并回退重建，
  最终成功；无 Rust 编译错误，不声称工具日志零警告。

此前关闭优化冻结状态单独通过 Store **409/0/1**，31.65s及workspace检查；其后按当前版本政策
删去旧迁移用例，所以不能用该旧数量冒充最终测试集。新增四项当前格式用例保留：空/Unicode/
多代次摘要正确、后续事务旧代损坏重新拒绝、旧代额外尾块拒绝、错误关闭摘要整事务回滚及正确
重试/幂等。只在函数固定快照内复用当前代次已核manifest元数据，不跨事务保存验证结论。
三次优化前/后三尺寸真实磁盘对照见 PERFORMANCE_BASELINE，32 MiB 关闭中位数由
3201.752ms 到 2280.675ms；不是整体请求或 UI 延迟的改善证明。

仍未全仓清除其他兼容路径：旧 capability descriptor 回退、权限旧拼写及无效配置默认、
空模型profile推断和未知provider猜协议已审计，尚待分别按当前协议收敛。不能据 Store 清理
宣称所有模块完成单版本整改。要求/Core结果/终态seal消费者仍未完整接线。

### 输入内容承诺与编码（2026-10-11）

Domain 三个位置已统一为 `RunInputCommitment`：要求集合 input、评估 assessed_input、结果
input；完整比较 watermark 与 applications_digest。旧仅数量 wire 拒绝，零水位只接受固定
空承诺，非零水位拒绝空承诺。RunResult 摘要分隔符升级 V2（不是 terminal seal V2）。
冻结后 `nola_cargo test -p nola_domain --lib --quiet`：**141/141**，0.11s，exit 0。
覆盖同长度不同摘要拒绝、完整承诺比较、旧格式拒绝、空前缀/溢出及结果摘要变化。
随后 `nola_cargo check --workspace --all-targets --quiet` exit 0，无编译诊断。
根代理另用 Ruby JSON/Digest 复算固定 result 与空承诺向量，分别为
`2b7dfabd0c7ccb443a8253b158f6db50cad8c61bacd477c23537b154c0bccd44`、
`e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd`，均相等。

仍无完整生产 requirement/seal 消费者；匹配调用者提供的两个摘要不证明来源完整。
封存前完整原应用覆盖、owner/决定 frontier 与来源解析仍需真实事务接线，不能从此项推导 E2E 成功。
同轮 Store 显式磁盘基准 1/1 通过，2/8/32 MiB 数据及构建身份见 PERFORMANCE_BASELINE 最新小节；
其使用前一轮 Store 二进制，不是本次 Domain 新协议性能结果，也不是 Mac UI/真实 Provider 验收。

### Run 输入来源前缀：Schema 66（2026-10-11）

后续来源完整性复核：迁移改为枚举原始 InputApplied，而不是旧派生索引。补“整组旧索引
缺失、原事件仍存在”反例，要求开库拒绝、schema 版本/新表回滚、原 accepted/event 保留。
冻结后再次执行 Store 全量：**405 passed / 0 failed / 1 ignored**，31.33s，exit 0；
ignored 仍为磁盘性能基准。以下 404 项是此前版本记录，不覆盖本次缺失组场景。

同轮架构审查补充了要求范围处置、控制局部引用、共同结果仲裁及 input commitment 目标契约。
它们尚未成为 Core/Domain 生产接线，不计功能通过；尤其普通正文绕过旧义务判定、三处 DTO
仅数量水位及新要求评估无生产事件的问题仍未解决。

冻结后的隔离数据库回归，经项目 mbx 脚本运行，设置 `RUST_MIN_STACK=16777216`；这不是默认栈验收。

- `nola_cargo test -p nola_store --lib --quiet`：**404 passed / 0 failed / 1 ignored**，30.79s。
  ignored 是需显式运行的磁盘性能基准，不是本次功能验收。新增五项覆盖跨 Turn 局部序号重置、
  单事件应用顺序、queued 排除、重放、原双来源损坏、真实 62→66 弱来源迁移，以及 schema65
  多余应用索引导致迁移拒绝、版本/新表回滚、旧事实保留。
- Application `terminal_event_provenance_tests`：**3/3**，0.27s；真实 Core→Application→SQLite
  首次准入、排队、重开、后继 promotion/replay。前驱 Run 摘要不变，后继独立从 1 开始，
  等数量不同输入摘要不同，不将排队输入计入前驱已应用范围。
- `nola_cargo test -p nola_application --lib --quiet`：**306 passed / 0 failed / 1 ignored**，5.86s。
  ignored 是进程终止测试的子进程入口；父测试负责启动该入口。此结果覆盖原始执行证据和等待回归。
- `nola_cargo check --workspace --all-targets --quiet`：exit 0，无编译诊断。

测试仅使用临时数据库/NOLA_HOME，不迁移用户库，不调用真实 Provider/Mac UI。
有界链头检查不是全历史完整性证明；RequirementScope/Assessment、结果绑定与 V2 seal 尚未接线。
未测长历史迁移、WAL 增长及生产延迟，不把单元测试耗时当作性能预算。

1. 类型/纯函数：不变量、状态可达、序列化错误。
2. 组件：真实 SQLite、Host、文件/进程回执与崩溃恢复。
3. 链路：客户端命令 → Application → Core → Host → Store → 客户端投影。
4. 真实模型：实际 wire、usage、工具执行与落库证据，不能用 mock 代替。
5. 实际交互：目标平台的窗口/键盘/滚动/会话切换与展示验证。

通过低层不自动覆盖高层。文档链接检查只证明文档可导航，不证明架构或业务正确。

### 当前恢复基础的执行记录（2026-10-11）

通过 `scripts/lib/rust_build_cache.sh` 的 `nola_cargo`（本机 mbx），分别使用新建临时
`NOLA_HOME`，未读取/改写生产数据库：

| 命令 | 本轮结果 | 实际能证明的范围 |
|---|---|---|
| `nola_cargo test -p nola_kernel --lib --quiet` | 122 passed，0 failed | 当前纯决策回归，含 effect/permission/recovery 的既有安全策略 |
| `nola_cargo test -p nola_application --lib run_recovery_port_tests --quiet` | 16 passed，0 failed，271 filtered out | MemoryMaintenance 接口 fixture 的恢复计划、身份/续租/游标/委托权限校验 |

两个命令均 exit 0；没有修改运行代码。结果不是新 RunResult 或受阻出口的验收，
也不是 SQLite 崩溃原子性、真实 Host 退出、Provider、UI 或性能预算证据。
当前实现的测试包含旧语义，测试通过不能推翻 A01/A07/A08 的架构缺口。

### 快照恢复原因整改（2026-10-11）

同样使用 mbx 和隔离 NOLA_HOME：`nola_cargo test -p nola_cli --bin nola compaction_ --quiet`
exit 0，34 passed，0 failed，1595 filtered out。
`compaction_snapshot_restore_preserves_failure_categories` 覆盖缺文件、不可读对象、损坏/缺字段、
32 MiB 超限、会话/配置不匹配和内层请求无效；现有 fork/fallback mock-provider 测试增加
`request_snapshot_malformed`，断言冷摘要完成、原因传递、usage 与不执行维护工具。
有效快照落盘恢复及输出计量保留也由该测试覆盖。
这只是 E2E-03/04 的 Host 层子集，不证明生产旧会话热分叉覆盖率，也不是实际 Mac 验收。

### Goal 源结果约束（2026-10-11）

`nola_cargo test -p nola_core --test agent_core_host_contract --test agent_goal_runtime_checkpoint_contract --test skill_goal_lifecycle_contract --quiet`
使用隔离 NOLA_HOME/mbx，最终 exit 0：分别 123、2、1 tests passed，无失败。
新增断言：从 checkpoint 恢复的 Completed Turn 若携带 Blocked/Incomplete/NeedsInput，
Goal acceptance 必须拒绝且 Goal 原值不变；合法问答接受仍无额外 Host 审查请求。
首次整批运行曾因续写测试检查旧提示词字面量而失败；已改为验证系统指令要求仅追加缺失后缀，
以及续写前后用户消息完全一致，重新运行后通过。未改变续写运行行为。
这是 Core/序列化级别，不涵盖 SQLite seal、子任务 receipt、客户端或完整新证据模型。

### 客户端回执与 CLI 整体基线（2026-10-11）

`nola_cargo test -p nola_cli --bin nola application_port::tests --quiet`：9 passed，exit 0。
新增真实 SQLite 验证写命令有 committed receipt，重开数据可读，只读查询不增加回执。
取消测试改为直接调用裸 Application API 检查外层事务要求，而非假设 CLI InProcess 绕过回执。
随后扩大到全 CLI：默认运行栈溢出；串行/16 MiB 测试栈运行结束为
1611 passed、15 failed、4 ignored，exit 101。详见 [完整失败基线](CLI_BASELINE.md)。
整体未通过；本轮不能据 9 项通过宣称客户端边界或 CLI 验收完成。

### Task 项目身份与通知复核（2026-10-11）

修改前，直接运行现有 CLI 测试产物的
`task_runtime::tests::single_terminal_result_primitive_preserves_independent_ui_delivery`：
0 passed、1 failed，exit 101。隔离数据库表明 UI 标记仍为 false，失败来自路径查询遗漏。
统一 Task 准入/查询的项目规范化后，使用 mbx、临时 NOLA_HOME、RUST_MIN_STACK=16777216：

- `nola_cargo test -p nola_store --test runtime_task_e2e --quiet`：30 passed，exit 0，2.21s。
- `nola_cargo test -p nola_cli --bin nola task_runtime::tests:: --quiet -- --test-threads=1`：
  10 passed、1 failed，exit 101，1.15s；原独立 UI 通知测试通过。
- 剩余跨会话测试不再失败于 owner task count，而是在手工 claim 时失败。
  隔离库已出现 cli-launch worker、running 和 worker_process_exited 事件：查询计数触发了 supervisor。
  尚未判定全部触发时序；不能放宽 claim 断言或认定跨会话链路通过。

以上不是重跑完整 1630 项后的新基线，不能简单从原 15 项失败中减掉某个数量。
通知提前 ACK 的故障窗口尚未修复，GOAL-05 的交付协议仍待实现和故障注入。

### Task 查询无执行副作用（2026-10-11）

移除隐式恢复及详情读取 ACK 后，在隔离 NOLA_HOME、16 MiB 测试栈运行：
`nola_cargo test -p nola_cli --bin nola task_runtime:: --quiet -- --test-threads=1`
结果 38 passed、0 failed、0 ignored，exit 0，18.66s；两个隔离子进程 fixture 也成功返回。
测试新增数据库观察：owner/foreign 查询与 pending context 不创建 attempt、不分配 worker、不离开 queued；
model read/UI detail 不改变 ui_notified。保留模型消费后独立 UI 通知的原断言。
覆盖该模块现有资源调度/Host 冷恢复，不覆盖所有 UI 无窗口/服务长期运行或默认小栈并发。
随后同设置跑完整 CLI：1615 passed、11 failed、4 ignored，exit 101，229.99s。
权限恢复一项只在完整套件报 lease expired，不能引用 38 项绿灯掩盖；
最新产物 hash、失败差异与 ignored 身份见 CLI_BASELINE 的最新复测节。

## 纵向场景矩阵

提交分段诊断（2026-10-11）：默认关闭的线程局部采集已接入 disclosure/SQL COMMIT/归档。
v3 实验 60/60 会话、510/510 父阶段、900/900 receipt 子阶段，连同 open/reopen 共 1020 条观察，
dropped=0；逐样本归档占首次正文回执比例 median 为新库 81.82%、复用库 82.46%。
这定位当前本地 fixture 的主要耗时，未实施性能优化，也不是生产 provider/UI 验收。
最终 `nola_cargo test -p nola_store --lib commit_diagnostics:: --quiet` 6 passed、exit 0，
包含真实 SQL 失败及归档失败但 DB 提交可恢复两条故障路径。
v3 采样产物早于 capture generation 加固，原始产物 digest 与后续测试证据分别保留；详见 PERFORMANCE_BASELINE。

Task 通知交付（2026-10-11）：Store `runtime_task_e2e` 30/30、CLI `notification`
16/16、真实 SQLite 独立 UI 交付专项 1/1、最终真实权限等待/unread 回归 1/1，均 exit 0。
后者验证模型 mailbox 已读不能过滤尚未交付的权限 occurrence；写入/flush 失败不 ACK。
Shell 回归覆盖清除进程内映射后重新恢复未确认终态，避免旧测试仅加载 live Task 的假阳性。
见 [精确范围](TASK_DELIVERY_IMPLEMENTATION.md)；未重跑完整 CLI，也未完成多端或真实终端 kill 验收。

Goal 语义变更失效（2026-10-11）：隔离 NOLA_HOME/mbx、16 MiB 栈，
`nola_cargo test -p nola_core --lib agent::goal:: --quiet` 最新复测 15 passed；
相关三组集成测试再次 123/2/1 passed，均 exit 0。
新增 scoped detail→依赖闭包失效、全局 constraint、普通 progress 不失效、
supersedes 旧/新作用域、批次拒绝原子性和序列化恢复保留证据的回归。
追加报告撤销验收时的拟提交状态检查：前置项降回 Pending 后不能遗留 Completed 依赖方，
拒绝原子性与合法整批撤销路径均已覆盖。
仅证明 Core Goal 语义与相关既有集成，不是新 RunResult 已落库或五端UI已通过。

本地主链实测（2026-10-11）：独立性能 harness 1 项通过，60/60 会话恢复精确正文，
510/510 阶段样本完整。新库/复用库准入 p95 为 71.03/69.51 ms，首次 1 KiB 正文回执
p95 为 52.26/52.33 ms。见 [原始证据与边界](PERFORMANCE_BASELINE.md)。
这是有并行构建竞争的 debug Application/SQLite 基线，不是 provider TTFT、UI 帧延迟或进程重启验收。

终态来源前置（2026-10-11）：Store 2 项真实提交/重开专项与 Application 1 项七变体专项通过。
见 [实现边界](TERMINAL_IMPLEMENTATION.md)。只证明历史 Core 决策能定位到原提交，
尚未接入 RunResult/V2 seal，不关闭终态纵向场景。

Goal 版本与拒绝原子性（2026-10-11）：隔离 NOLA_HOME/mbx、16 MiB 测试栈。
`nola_cargo test -p nola_core --lib agent::goal:: --quiet` 11 passed；
`nola_cargo test -p nola_core --test agent_core_host_contract --test agent_goal_runtime_checkpoint_contract --test skill_goal_lifecycle_contract --quiet`
分别 123/2/1 passed；CLI 构造点以 `nola_cargo check -p nola_cli --bin nola --quiet` 编译通过。
新增报告版本绑定、工作更新/恢复后的旧报告拒绝、验收后修改不能沿用旧报告完成、
重复 fact/artifact 注册不覆盖原值。新字段无默认，旧审计报告缺字段明确拒绝。
这些是 Core/序列化与 CLI 编译证据，不代表版本切换、真实模型重评或多端验收完成。
追加恢复出口断言后，精确运行
`nola_cargo test -p nola_core --lib agent::goal::tests::audited_state_transition_can_complete_goal --quiet -- --exact`，
1 passed、446 filtered、exit 0；验证显式新契约重评后可以完成，不以旧报告直接消除冲突。

封存结果 Domain 契约（2026-10-11）：`nola_cargo test -p nola_domain --lib --quiet`，
隔离 NOLA_HOME/mbx，116 passed、0 failed、exit 0，0.11s；`git diff --check` 通过。
其中新增 6 项 RunResult 测试覆盖六种结果 round-trip、禁止等待终态/混合字段/默认成功、
必需引用与未完成项、重复和冲突 effect 引用、持久整数边界、超过 JS 精确整数范围的无损编码。
结构校验不查询数据库，不能证明 evidence/acceptance 引用存在或归属正确。
生产 Core→Application→Store→seal→客户端尚未消费该类型，E2E-01/02/13 均未因此关闭。

随后完成 RunResult canonical bytes/digest：同环境重跑 Domain 全部单元测试，
119 passed、0 failed、exit 0，0.11s，`git diff --check` 通过。
新增 3 项测试核对集合排序与正文顺序区别、identity/frontier/reason/evidence 改动影响摘要、
独立 Ruby 生成的固定编码/摘要向量。不是远端协议升级验证；旧 seal 仍未绑定 RunResult。
TERMINAL_PROTOCOL §4.1 已逐项区分已有来源与尚缺的 requirement/acceptance/obligation 持久身份。

等待语义反例复核（2026-10-11）：隔离 NOLA_HOME/mbx、16 MiB 测试栈，执行
`nola_cargo test -p nola_core --test agent_core_host_contract user_question_reaches_host_without_fabricated_sop_metadata --quiet -- --exact --test-threads=1`，
1 passed、122 filtered、exit 0，0.03s。该测试明确断言 unavailable 提问生成
`TurnCompleted { outcome: NeedsInput }`，因此结果证明 A16 所述当前行为，不证明目标等待契约通过。
目标行为、事务故障点及多问题顺序覆盖见 WAIT_PROTOCOL；未修改生产等待语义或旧测试预期。

Goal 来源/评估方式分离（2026-10-11）：隔离 NOLA_HOME/mbx、16 MiB 测试栈。
`nola_cargo test -p nola_core --lib agent::goal:: --quiet` 对应 9 项通过。
实际首条命令还列了三个 integration target，但 filter 令它们各运行 0 项，不能算验收；
随后去掉 filter 单独执行 `--test agent_core_host_contract --test agent_goal_runtime_checkpoint_contract
--test skill_goal_lifecycle_contract`，分别 123/2/1 通过，exit 0。
覆盖普通接受记录 ModelJudgment、source_verified、无 independently_verified 输出、缺方法拒绝、
序列化 checkpoint 恢复审计记录相同；没有验证旧用户数据迁移或完整 requirement acceptance 协议。

收尾前置重构（2026-10-11）：统一两条 Application 写路径的 checkpoint 生命周期映射后，
隔离 NOLA_HOME/mbx、RUST_MIN_STACK=16777216 下运行
`nola_cargo test -p nola_application --lib --quiet -- --test-threads=1`：
288 passed、0 failed、0 ignored，exit 0，6.41s。
新增纯映射测试覆盖九个 Core 生命周期；整库回归不等于新结果协议/SQLite seal/多端 UI 已实现。

依赖边界检查（2026-10-11）：隔离 NOLA_HOME/mbx 下，
`nola_cargo test -p nola_cli --test dependency_boundary --quiet` 3 passed；
`nola_cargo test -p nola_domain --test workspace_dependency_architecture --quiet` 14 passed。
两组 exit 0、测试阶段各 0.03s。详见 DEPENDENCIES；这是图/源码边界子集，不代替下表纵向场景。

| ID | 场景/故障注入 | 必须断言的结果 | 当前证据状态 |
|---|---|---|---|
| E2E-01 | 最新真实长会话的变更+验证+未知范围 | 有类型证据限制，可退出 blocked，不循环生成，不谎报 completed | 有失败反例，尚无修复通过证据 |
| E2E-02 | 已知范围写入、旧版本检查、未知写后版本、真实业务检查失败 | 四类证据不混同；正确版本才能完成；受阻保留产物 | 部分单测存在，需整链路映射 |
| E2E-03 | 无/坏/不匹配快照的长历史冷恢复 | 原文完整、可继续、维护成本可见、结果可封存 | 最新生产能压缩，整轮失败 |
| E2E-04 | 热压缩、真实截断、续写空结果/工具调用/取消 | 保前缀；仅完整摘要提交；失败保原文 | 专项通过部分，不代表全部组合 |
| E2E-05 | Ask/Allow/Deny、OS 拒绝、授权期间撤销、自动批准 | 准确等待/拒绝/执行，无越权，无不必要重问 | 待逐端复核 |
| E2E-06 | 发送、提交 ACK 丢失、双击、重复消息 | 同 command_id 只有一个 Run/Assistant，无重复外部效果 | 待复核 |
| E2E-07 | 派发前/中/后、提交前/后 kill Host/Service | 恢复已提交事实；未知写入不盲目重跑 | 待按全部事务边界复核 |
| E2E-08 | 执行中切换会话、结束后跟随、重开窗口、删除 Trace | 新会话输入独立、无闪烁/空白、canonical 可恢复 | 待五端真实 UI 证据 |
| E2E-09 | Goal/subagent 跨压缩与重启、父取消、子失败 | 约束/进度/证据持久，权限不扩大，子成功不等于父完成 | 待复核 |
| E2E-10 | 多端点配置、会话覆盖、凭据失败、模型切换 | 唯一有效快照，旧消息不变，无凭据泄漏 | 待四路配置接线复核 |
| E2E-11 | Memory 禁用、来源撤销、数据源过期、后台重复触发 | scope/freshness 准确，自动化复用标准 Run | 待复核 |
| E2E-12 | 远端断线、重复乱序、跨设备授权/同步 | 复制不产生执行权，精确归属与可恢复投影 | 待复核 |
| E2E-13 | 用户提问、并行工具两种结束顺序、多客户端答复、等待期间断连/取消/重启 | Open wait 不封存成功；准确区别 unavailable/Decline；回答幂等、无模型轮询、旧回答不复活 Run | Schema64/Core→Application→SQLite、CAS、Task 调度、取消提交顺序、问题及答复提交后 SIGKILL 已测；五端真机/Provider/提交中崩溃与完整父子 Task 链仍待测，见本页持久用户问题等待验收 |

### 要求与逐项评估的可执行契约（2026-10-11）

`nola_cargo test -p nola_domain --lib --quiet` 冻结后 **129 passed / 0 failed**，0.11s。
隔离临时 NOLA_HOME、mbx、16 MiB 测试栈。新增 `run_requirements` 六项测试与
`run_result` 两项 coverage 测试，覆盖要求唯一身份/版本/水位/来源、依赖 DAG、三种
判断来源、严格 wire，以及行动结果不得只接受成功子集、重复接受同一要求、引用未来或
其他范围的评估。同一 Core session 的后续 Turn 可以提供评估，实际 Run 归属须由 Store 证明。

首轮128通过/1失败暴露 serde 内部标签 unit variant 会忽略混入字段；将 ModelJudgment
改为空字段 struct variant 后严格拒绝伪混合用户命令字段，最终重跑全 Domain 通过。
ModelJudgment 不是独立验证；UserAcceptance 的命令引用不是认证证明。
coverage 只校验传入集合，不能发现数据库中被省略的更新评估或伪造引用。
Core 产出这些事件、Store 来源索引和最终 seal 仍未接线，本结果不关闭 E2E-01/02/09。

## 门禁与整改交付

每行验收需记录：契约 ID、当前具体测试入口、输入 fixture、故障点、观测字段、预期终态、
实际产物/状态、执行命令、环境、结果和未覆盖范围。不能只登记一个测试函数名。
现有入口如 `scripts/test_real_mac_compaction.sh` 只对应 E2E-04 的子集；
`scripts/test_mac_layers.sh full` 不能自动证明其它四端 UI 或真实 Provider 场景。

完成条件：模块依赖审计完成；LC 状态与 DTO 已映射到实现；冲突文档修订；各模块有对应测试；
本表必需链路通过；性能测量与预算定版；生产故障在隔离副本/脱敏同构 fixture 中可复现并通过。
若保留未验证项，Goal 必须保持进行中，而不是将范围缩成“文档已写”。

## Schema 63 输入事实索引专项（2026-10-11）

范围：初始/活动生产事务内的接受、Turn-local 应用、撤回索引及跨 Run 来源绑定；不是完整
RunResult watermark 或 V2 seal。实现边界和历史数据限制见
[TERMINAL_IMPLEMENTATION](TERMINAL_IMPLEMENTATION.md) 的“Schema 63”小节。

- 隔离 NOLA_HOME、mbx、16MiB 测试线程栈；Store 全库命令
  `nola_cargo test -p nola_store --lib --quiet`：383 passed / 0 failed / 1 ignored，34.36s。
  包含六项 `input_facts::tests`：重开、原事务回滚、接受/应用序号缺口、重复 item、digest 形状、
  错误 session/revision/cursor、超限元数据、撤回后应用、原事件身份篡改与 62→63 来源回填。
- 真实 Application（本地 SQLite、无外部 Provider）命令
  `nola_cargo test -p nola_application --lib terminal_event_provenance_tests -- --nocapture`：
  3 passed / 0 failed，0.14s。核验初始与活动两条生产链跨重开持久 3 accepted / 1 applied，
  未应用 Next 不误入旧 Turn 的应用账本；同时覆盖独立后继 Run 的原 admission 来源。
- 历史回填标记 `historical_event_only`，不授予当时 frontier 或封存信用；完整历史应用缺少
  admission 的迁移目前 fail-closed，可能阻止开库，没有隐式丢数据或 catch-all 跳过。

首次 Store 回归 377 passed / 3 failed / 1 ignored：两项后继 outbox fixture 缺请求字段，
一项旧版本 schema 比较未列出新增对象；修正 fixture/精确对象集合后完成上述全库复跑，未放宽
业务断言。不改写归档 schema 62/v4 性能证据，也不据这组结果宣称生产 UI/Provider 已通过。

补充并发准入验收：两个独立 Store/SQLite 连接由两个线程同时请求同一后继，覆盖相同请求
幂等返回和不同请求确定冲突；重开核验唯一 Run、绑定、输入应用、消息、事件和 outbox，以及
完整性/外键检查。`successor_admission` 专项 4/4 通过；加入此例后的 Store 全库复跑为
384 passed / 0 failed / 1 ignored，29.26s（同上隔离环境与测试栈设置）。这是同进程独立连接
竞争测试，不代替双进程强杀恢复验收。

迁移发布风险：候选旧事件当前按 `core_envelope.session_id` 为文本筛选，选中不等于完整契约。
缺 revision/cursor/applications、应用缺接受来源、应用序号缺口或身份冲突均可能阻止开库。
回滚保留 schema 62，不自动修复或隔离坏会话。迁移在单次写事务中收集候选事件 ID 后逐行读取；
长历史耗时与峰值内存尚未测，也没有在用户现存库上验收，不能由正常小 fixture 推断可直接发布。

CLI 最终冻结回归：`nola_cargo test -p nola_cli --bin nola permission_recovery_entrypoint_tests
-- --nocapture`，17/17 通过（10.94s）；独立 Host 状态重置专项
`successor_boundary_resets_host_run_state_but_preserves_cancellation_scope`，1/1 通过。
真实本地 HTTP/SSE + SQLite 用例覆盖热运行与关闭重开：两次物理请求、后继文本不进入首请求、
独立 Run 持久终态、exact Run 不接管后继、已取消恢复不准入后继、通知引用新 Run seal。
Host 重置专项核验旧临时状态清除且同一前台取消范围保留。最初新 seal 读取发生 Rust E0502
编译错误，改为先捕获 owner 再可变读取 Store 后完成上述回归；没有放宽运行时 owner 校验。
仍未覆盖取消恰在 successor 提交瞬间的确定性故障注入、实际进程 kill、真实 Provider 和 Mac UI。

### 持久用户问题等待：Schema 64 验收（2026-10-11）

全部使用隔离临时 `NOLA_HOME`、`nola_cargo`/mbx 与 `RUST_MIN_STACK=16777216`；
未使用生产数据库、凭据或续跑用户会话。测试为真实 Core/Application/SQLite，不是真实 Provider。

| 命令/范围 | 实跑结果 | 证据边界 |
|---|---|---|
| `nola_cargo test -p nola_store --lib` | 393 passed / 0 failed / 1 ignored，30.81s | Schema64 时点全库；含6项 wait 索引/回滚/重开/CAS/旧schema无假问题及3项 Task 等待测试。其后 Task helper barrier 补丁另由负责代理验收，不能将此结果算为后续产物全库复跑 |
| `nola_cargo test -p nola_application --lib user_input_wait_tests -- --nocapture` | 最终5 passed / 0 failed，0.65s；291 filtered out | 真实模型 Host 事件产生问题，无问题 Host effect；重开 List，合法答复只新增一次 continuation，终态后原命令 Replay，不同内容 Conflict；错误 owner/schema/option、两个独立连接竞争、取消后 late answer 均覆盖 |
| 同上 Taskchild 专项 | 包含在5项中，最终通过 | 从可信 receiver admission 导入开始，真实 fair claim→App lease 准入/Run binding→等待退役→答复 queued/task_scheduler；pending outbox 未 ACK，无新执行 attempt，随后 fair claim 取得新 attempt 且 Run binding 不变 |

索引审查：问题内容由不可变 `run_events` 持有，不在 head/response 索引重复保存正文；
回答与 source submission digest、Core frontier、canonical owner 绑定。原事件重复索引幂等，
head CAS 和唯一 response 约束防双答；事件/回答/checkpoint/outbox/Task 状态共用提交事务。
取消关闭 wait 后才可封存，失败注入保留原开放问题且无孤立回答事实。
Applied/Replay 的 resume_policy 来自持久 Task repository 查询，不按会话 ID 字符串猜测。

首次编译发现 App crate 路径、窄 trait 和新等待状态的穷尽映射未接齐；补齐后4项通过，
再增加 Taskchild 政策与 pending outbox 断言，最终5项通过且无该测试的编译 warning。
中间一次并行编辑触发 mbx 输入变更警告、未缓存对应产物，最终复跑覆盖更新后代码。

残余边界：无真实父委派传输认证/父子 Task 全链、无真实 Provider/五端窗口交互、
无实际多进程 kill 与 ACK 丢失注入；双连接竞争和关闭重开不代替这些验收。
等待首帧与答复后续延迟尚未采满30样本；此批测试耗时不是性能 SLO。

父 Task 问题查询与混合工具批次补充：同命令最终 **8 passed / 0 failed**，1.16s，
291 filtered out。新增真实 SQLite 用例覆盖父 owner/task 精确过滤、保留原 child 身份、
没有 immutable binding 的 Task 不可读、resolved/取消意图/Core 取消终态排除；查询不会
修改原 wait。`ListOwnedTaskUserInputWaits` 与既有 List/Load wait 均明确列入只读命令分类。
同批 question + independent read 的真实 Host 回执用例核验 wait 已开放而 Task 仍 running，
原 lease 保留；独立 read 完成并提交 waiting_input 后才退役 attempt。没有以手写 checkpoint
证明该顺序。此组不覆盖 GUI 展示、父问题通知 ACK 或真实进程崩溃。

父问题通知 Store 最终补充：`nola_cargo test -p nola_store --lib
runtime_task_user_input_wait -- --nocapture` **5 passed / 0 failed**，0.15s。
覆盖独立工具完成前保留 executor、事务回滚、取消不伪造答复，以及精确 question
occurrence 的独立 consumer ACK、幂等冲突、过期问题拒绝新增 ACK、关闭重开持久化。
关闭重开不是进程 kill；本次没有重跑 Store 全库，不能覆盖写为全库通过。

CLI 父子问题交互最终冻结后复跑：`nola_cargo test -p nola_cli --bin nola
user_input_wait::tests -- --nocapture` **6/6**；同产物
`waiting_task_question_read_failure` **1/1**。覆盖子身份不可被父替代、部分提交后
后续冲突/错误/Esc 保留父恢复候选、仍有开放父问题时不恢复、Esc 不当答案，及
问题查询失败不可吞为空成功。此前通知定向 16/16、SQLite 独立交付 1/1；
没有真实 PTY 父子全链、真实 Provider、kill 或取消竞争验收。

取消顺序及真实进程终止补充：冻结后再次运行 Application `user_input_wait_tests`，
**10 passed / 0 failed / 1 ignored**，1.16s。ignored 是仅供父测试启动的 worker 入口，
不是跳过恢复验收；父测试实际启动两个子进程，分别在问题提交、答复提交后保持 SQLite
连接并被 SIGKILL，确认退出 signal=9。重开保留同一问题、原回答 Replay、同一条
continuation request ID，没有第二次模型 effect。固定 Core Host 输入，不调用真实 Provider。

新增取消测试分别经 stateful Runtime 与 stateless dispatcher 验证取消先于答复、答复先于
取消：先取消返回 `cancellation_requested` 且事件/答案/outbox/revision 不变；先回答则原
命令保留 Replay，取消后不能 fresh claim。这是两个持久提交顺序，不是并发 Host 正在写文件
时的取消验收。CLI 同批四种 Conflict 原因文案定向 **7/7** 通过。
首轮编译期间 worker 标记有变更，原11项结果不作为最终计数；上述是冻结后重跑结果。
仍缺事务提交中 SIGKILL、真实工具副作用、断电及五端窗口整链；当前测试不能证明这些边界。

### Mac / CLI / Android / Web 等待接线验收（2026-10-11）

- Mac：`xcodebuild test -scheme NolaMac -configuration Test -destination
  'platform=macOS,arch=arm64' CODE_SIGNING_ALLOWED=NO`，仅运行
  `MacRunCoordinatorTests`、`MacRemoteSessionProjectionEventTests`、
  `MacAgentHostDecoderCooperativeTests`、`MacChatInteractionBoundaryTests`。
  冻结代码后的最终结果 **40 passed / 0 failed / 0 skipped**；没有替换 Products/Debug。
  最终结果包：`Test-NolaMac-2026.10.11_7-08-59-+0800.xcresult`；日志
  `/tmp/nola-wait-mac-frozen-final.log`。已有 headermap 构建警告仍存在，不声称零警告。
- Mac 覆盖 Core/session 与 conversation/control 身份区分、开放/关闭问题解码、
  稳定答复身份匹配、waiting_for_input 独立活动状态、exact Run 等待/取消/恢复、
  EOF 回答竞态，以及生产问答卡片在紧凑滚动容器中的布局。
  核对 `/tmp/nola-ui-remediation-options-{initial,scrolled}.png` 后修正了原生
  bordered 按钮实际裁掉多行说明、但 AX 暴露全文导致旧断言误通过的问题；
  新断言要求选项本体为六行说明保留实际高度，不只检查 AX 字符串。
- CLI：user_input 7/7、desktop_host_control 9/9、question 5/5、
  pending Core pump 1/1；Store Task 等待 3/3。过滤集合可能重叠，不相加当独立测试总数。
  最后取消递交分支修正后 CLI check 通过。
- Android：`ConversationProjectionV1DecoderTests` 定向 Gradle 测试通过，
  包括新增 WaitingForInput；Web 新 user_input_wait_projection 与既有 current_tool
  两项 Node 测试通过。此处不是 Android 真机问答表单验收。

仍需真实模型问答、跨进程 kill/ACK 丢失注入和父 Task 主动投递问题的完整验收。
Mac 的字典重试身份及读取失败恢复已接线，但不能用静态 intent 匹配测试代替真实
断网/服务崩溃后的回执恢复测试；这些保持未完成。

### Android 冻结后统一 JVM 验收（2026-10-12）

- 命令（`android/`）：`./gradlew :app:testDebugUnitTest --max-workers=2 --console=plain`。
  **exit 0，BUILD SUCCESSFUL，1m 34s**；30 个任务，18 executed / 12 up-to-date。
  本次实际完成 `compileDebugKotlin`、`compileDebugJavaWithJavac` 及测试源码编译。
- XML 报告位于 `android/app/build/test-results/testDebugUnitTest/TEST-*.xml`：
  103 个 suite、400 个 test，**398 passed / 0 failures / 0 errors / 2 skipped**。
  skipped 为 `OpenAiProviderRealEndToEndTest` 的两条真实模型测试，要求显式设置
  `NOLA_REAL_LLM_E2E=1` 及 provider 配置；未将其计为通过或声称真实模型已验收。
- Gradle wrapper 8.9，daemon 按项目 pin 使用 JDK 21，Android 字节码目标 17。
  发现本机失效 `temurin-8.jdk` 的 toolchain 探测警告，但未阻断构建；未修改全局 Java 安装。
- 未运行 `assembleDebug`：其 `configureCMake` 依赖 `buildRustCore`，会与主验收 Cargo
  构建争锁；本批只声明 Kotlin/Java/JVM 编译测试成功，非 native JNI/APK 打包或设备验收。
  未连接、轮询或操作物理设备；本次 Android 无需修复源码。

### Web 与测量导出 / 报告冻结后统一验收（2026-10-12）

- Web 工作目录 `web/frontend/`：`npm run check` 最终 **exit 0，0 errors / 0 warnings**；
  `npm test` 最终 **exit 0，289 passed / 0 failed / 0 skipped / 0 cancelled**，19.407s。
  完整执行 package 中的 Node 测试集合与两项 Web locale 测试入口，不是定向过滤。
- 首轮 Web check 为 0 errors / 1 warning；首轮测试 285 项，283 passed / 2 failed。
  本批修复 `ChatResponsivePanel.svelte` 的非响应式 focus 引用；
  `chatResponsivePresentation.test.ts` 的组件计数误匹配 TypeScript 泛型；
  `conversationCacheReader.ts` 的 constructor parameter properties 改为等义显式字段，
  使生产模块可由 Node strip-only 测试入口加载。该入口原来加载失败，修复后实际执行
  文件中的测试，因此最终总数为 289，不将前后两轮相加。
- `npm run build`：**exit 0**，Vite 8.3.0，273 modules，354ms。
  产物 `web/app/static/ui/index.html` 及 `assets/`；入口 JS 410.88 kB（gzip 141.87 kB），
  Chat chunk 131.10 kB（gzip 37.83 kB）。index SHA-256：
  `ad3f2fb6b58cbf292f79960c29b36a7c1394398535410ce54c7a5a518dea59e1`。
  Svelte 插件提示未发现独立 config、使用默认配置；未作为错误或隐藏该提示。
- Repo 根目录：`python3 -m unittest discover -s scripts/tests -p 'test_*measurement*.py' -v`，
  **exit 0，27 passed / 0 failures / 0 errors**，0.005s；包括 exporter 4 项及 report 23 项。
  此前模块式命令 `python3 -m unittest scripts.tests.test_export_measurement_receipts scripts.tests.test_report_measurement_events -v`
  因 `scripts.tests` 非可导入 package 产生两个 loader error、exit 1，未执行真实测试；
  改用上述 discover 命令后完整通过，不记为产品失败。
- Python 使用隔离内存 SQLite fixtures；未读取用户数据库、密钥或调用真实 Provider。
  本批证明源码检查、自动化契约测试及生产静态打包通过，不证明浏览器布局 / 焦点实测、
  真实模型首 token、UI frame 延迟、缓存命中率或压缩性能基线达标。

### Web Automation typed execution 接线后验收（2026-10-12）

- 与 Rust 同步 `executionSpec`（内部 snake_case Agent spec）、`occurrenceKey`、
  `expectedRevision`、`occurrenceId` 和 canonical receipt 六态。创建必须明确 prompt、
  workspace root、permission profile；当前 Host 不支持的 context/skill refs 明示为空。
  来源/脚本标注为展示 metadata，不能被误认为绕过 Core 直接执行。
- run-now 发送前将原 occurrence intent 保存到 sessionStorage；未确认响应、刷新和重试
  保持同一身份。202 不宣称完成，只有绑定 revision/occurrence 相符的 receipt 才展示
  answered / action_completed / incomplete / blocked / cancelled / failed 终局。
- `web/frontend`：`npm run check` **exit 0，0 errors / 0 warnings**；
  `npm test` **exit 0，292 passed / 0 failed / 0 skipped**；
  `npm run build` **exit 0**，Vite 生产构建完成（620ms）。
  定向 automation helper/实际 Svelte SSR rendering **13/13**，属于上述完整集合，不相加。
- 测试覆盖显式权限与工作区、参数协议、重试身份、revision 冲突、六态 receipt 和七语言。
  未运行真实浏览器、真实自动化模型或端到端后端执行；本段不把 SSR 视作交互实测。

### Web Automation/Results 原会话导航收尾（2026-10-12）

- Results detail 使用 canonical `conversationId` / `runId`；Automation 的 pending
  observation 或 terminal receipt 必须匹配原 occurrence 和 definition revision，
  才提供原会话链接。两个不透明 ID 均精确编码保留，缺失绑定不推断新会话。
  只增加导航，不改 Chat 授权/恢复逻辑，不将 202 变成完成。
- `cd web/frontend && npm run check && npm test && npm run build` **exit 0**：
  Svelte **0 errors / 0 warnings**；**294 passed / 0 failed / 0 skipped**；
  Vite 生产构建 **1.23s**。测试输出 `/tmp/nola-web-final-navigation-tests.log`。
  新增精确身份编码、缺字段拒绝、pending/terminal 绑定和 revision/occurrence
  不匹配反例；未执行浏览器点击或真实模型自动化，不宣称这部分 E2E 已通过。

### Mac contracts 隔离验收首轮（2026-10-12）

- 使用 `NOLA_MAC_DERIVED_DATA=/tmp/nola-mac-contracts.SuNCkV/DerivedData`
  及同临时目录下的独立 `NOLA_XCTEST_DEVICE_SET` 执行
  `./scripts/test_mac_layers.sh contracts`。脚本强制 `-configuration Test`、
  `platform=macOS,arch=arm64`、`CODE_SIGNING_ALLOWED=NO`；没有覆盖签名 Debug、
  操作物理 iOS 设备或清理用户原有 XCTest 克隆。
- **exit 65，测试执行 0**：`NolaMac/Services/MacActivityStore.swift:8` 的
  `nonisolated typealias MacRunCompletionStatus` 编译失败，修饰符不适用于 typealias。
  日志末尾 3 failed build commands 是同一诊断导致的编译与 test 取消，不是三条测试失败。
- 日志 `/tmp/nola-mac-contracts.SuNCkV/contracts.log`；xcresult 位于该隔离
  DerivedData 的 `Logs/Test/Test-NolaMac-2026.10.12_10-04-28-+0800.xcresult`。
  本轮仅诊断，等待修复后再进行 contracts 测试；全局测试锁已正常释放。

#### Native Host 与 UI 交接的最小补充验收组合（待执行）

- `contracts` 包含 `MacUnknownEffectRecoveryControlTests` 的原 receipt、冻结
  intent、错误 owner、持久化失败等边界；不包含以下真实 helper 进程路径。
- `cancel-e2e` 层使用 bundled cancel-only 进程及隔离数据库，验证未派发 Run
  的取消 seal，以及错误 assistant 身份不修改数据库。
- 定向 `MacAgentCapabilityEndToEndTests/testWholeAppColdPermissionRecoveryRebuildsOriginalRunAndCommitsItsSeal`：
  本地 mock provider 加真实 Host 进程；safe 写权限等待后仅杀死测试持有的 Host PID，
  重开数据库，验证错误 Run/workspace/credential 被拒且 checkpoint/digest 不变；
  原 permission ID 单次批准后读回文件，并取得原 Run 的 actionCompleted seal。
- 同 suite 的 `testConversationPermissionIsReusedOnlyForMatchingBashScopeAcrossAgentHostRuns`、
  `testOncePermissionDoesNotPersistAcrossAgentHostRuns`、
  `testConversationPermissionDoesNotLeakIntoAnotherConversation` 补作用域隔离。
- 真实 provider 可进一步跑 `scripts/test_real_mac_permission_approval.sh`、
  `scripts/test_real_mac_permission_denial.sh`、`scripts/test_real_mac_cancellation.sh`，
  以及 `MacRealProductUIEndToEndTests/testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible`。
  必须使用隔离 Test/工作区/数据库并串行 XCTest；脚本跳过不能视为通过。
- 明确缺口：冷恢复测试只重建 runtime/port，不是整个 GUI 进程重启；mock provider
  不证明真实模型行为；权限卡呈现计数不证明鼠标/键盘交互或滚动视觉性能。
  上述是现有入口的只读核查与后续计划，尚不构成已执行证据。

### Mac contracts 隔离复验第二轮（2026-10-12）

- 同一隔离 DerivedData/Test 与 XCTestDevices，执行 contracts 脚本；日志
  `/tmp/nola-mac-contracts.SuNCkV/contracts-2.log`，**exit 65，测试执行 0**。
- 首轮 typealias 修饰符错误已消失；唯一编译诊断为
  `NolaMac/UI/MacAutomationCenterView.swift:304` 对 `MacActivityEntry.Status` 使用
  已删除的 `.completed`。静态审查同文件 317 行还有相同使用点；两处共同驱动
  `latestRunCompleted` 的绿色勾选图标，后续必须依据 typed 结果明确成功语义，
  不可恢复旧枚举或将 executionFinished/所有终态统一视为成功。
- xcresult：隔离 DerivedData 下
  `Logs/Test/Test-NolaMac-2026.10.12_10-19-51-+0800.xcresult`。
  本轮只收集失败及使用者诊断，未边修边重测，guard 已释放给 iOS 验收。

### Mac contracts 隔离复验第三轮（2026-10-12）

- 同隔离 Test/DerivedData contracts：编译通过，最终 **exit 65**。
  XCTest 执行 **989 tests，2 skipped，170 assertion failures（103 unexpected）**；
  按日志 Test Case 终态统计 **79 个失败用例**，不能将 170 条断言当作独立缺陷。
  测试阶段约 177 秒，日志 `/tmp/nola-mac-contracts.SuNCkV/contracts-3.log`。
- 首要共同根因：`MacAssistantJournalTestFixture` 的 Core `turn_identity` 缺少
  model capability profile/revision；90 条断言直接指向 fixture，另有本地运行、
  提交准备、远程取消/权限的空 continuation、空 HTTP 数据等连锁失败。
  提交准备日志明确先出现 fixture 准入失败，尚不能据此断言生产配置路径失败。
- 独立分组：签名测试仍检索旧 pbxproj 内联 codesign 而生产已提取脚本；
  本地化两项覆盖失败（16 key / 108 language entries，另静态覆盖重复报告
  stop_testing 六语言）；权限 guidance 按钮 accessibility 查找失败；一项空 journal
  测试报告 idle→failed(deinit)，需核失败清理路径，不删除终态断言。
- 所有结果先收齐，未边修边重测；尚未运行 host-e2e/真实模型补充组合。
  guard 正常释放，交给后续 iOS 验收。

### Mac contracts 隔离复验第四轮（2026-10-12）

- 同隔离 Test/DerivedData 与 XCTestDevices，完整 contracts，**exit 65**。
  编译通过；XCTest **991 tests，2 skipped，17 failed cases / 32 assertion failures
  （0 unexpected）**，约 157 秒。另 Swift Testing **29/29 passed**，不混入 XCTest 计数。
  日志 `/tmp/nola-mac-contracts.SuNCkV/contracts-4.log`。
- 15 用例 / 30 断言：`MacLocalRunTerminalContractTests` 的独立 fake runtime
  `run()` 直接 begin_assistant_turn，190 行 core_config 未声明 model capability；
  这是另一 fixture，不是上一轮已修的共享 journal fixture。
- 另两项：`MacInspectorInteractionTests:109` 缺 guidance.open accessibility 元素；
  `MacWorkspaceSteeringJournalTests:119` 仍在结束后的原 Run claim 后继 model 请求，
  需按显式新 Run promotion 契约更新 fixture，不能恢复旧 Run 内自动接续。
- 上轮共享 journal fixture、签名契约、本地化覆盖错误已消失。本轮完整收齐后
  未修改源码或重测，guard 已释放；真实 Host 进程补充组合仍未执行。

### 2026-10-12 Mac contracts 第五轮（统一源码冻结后）

- 命令：`NOLA_MAC_DERIVED_DATA=/tmp/nola-mac-contracts.SuNCkV/DerivedData NOLA_XCTEST_DEVICE_SET=/tmp/nola-mac-contracts.SuNCkV/XCTestDevices ./scripts/test_mac_layers.sh contracts`。
- 日志：`/tmp/nola-mac-contracts.SuNCkV/contracts-5.log`；exit 0，`TEST SUCCEEDED`。
- XCTest：992 项，2 项跳过，0 failures / 0 unexpected，163.346 秒；另 Swift Testing 29 项、5 suites 全部通过。
- 两项跳过为显式 opt-in 原生目录选择器和需要私有一致数据库备份的长会话回放，不计作已验收。
- 已通过真实 Canvas 权限指导按钮/草稿生命周期、独立 LocalRun terminal fixture、Steering 旧 Run 封存与新 Run promotion；新增实时用例经生产 canonical projection subscription 收到四条消息，不调用 reload，不继承父远程身份。
- 警告仍有 Rust target 超过 24 GiB、NolaMac/NolaMacTests/NolaMLXRuntime 的旧式 headermap 配置。系统日志包含 Apple linkd.autoShortcut 连接告警，但没有对应测试失败。
- 全程 Test 配置与隔离 DerivedData，未覆盖签名 Debug，未清理用户数据；本轮未边测边改，完成后 guard 已释放并通知 iOS6。额外真实 Host 冷恢复/权限/取消组合仍待执行，不能由 contracts 全绿替代。

### 2026-10-12 原 Run 封存后冻结配置读取验收

- 修复仅接受当前 `sealed` Run 契约：Store 在同一 pinned snapshot 读取 canonical terminal result；Application 核对身份、digest、timestamp、checkpoint decision/Turn，不以旧状态名或无证据布尔值认定终局。
- 第一轮 `exact-sealed-reader-1.log`：真实 answered/failed/cancelled 三类封存冷读通过；旧 fixture 两处失败分别为 cancelled 标签断言和删除被不可变证据触发器阻止。完整收齐后集中修 fixture，不更改生产约束。
- 最后一轮隔离 `NOLA_HOME` / `NOLA_DB_PATH`、16 MiB 栈执行 `mbx test -p nola_application --test run_invocation_recovery_e2e --test run_recovery_checkpoint_identity_e2e --test run_recovery_inspection_e2e --no-fail-fast -- --test-threads=2`：**exit 0，8/8 + 6/6 + 3/3，共 17 项通过，无跳过**。日志 `/tmp/nola-final-verification.gMkJRJ/exact-sealed-reader-2.log`。
- 反例保留缺 seal、外来 assistant 拒绝及不可变 SQL 删除拒绝；真实记录经窄 Repository 正控制可读后，仅去除 commit witness 必须拒绝，未禁用任何生产 trigger。
- 同生产源码 `mbx check --workspace --all-targets` **exit 0**，日志 `/tmp/nola-final-verification.gMkJRJ/workspace-sealed-reader-check-1.log`；其后只改上述两项 fixture。Mac 真实 helper 由独立验收记录提供，不由这些 Rust 测试代替。

### 2026-10-12 Mac scheduled resume 来源隔离第二轮

- 全批源码冻结后执行 `mbx test -p nola_host_protocol --lib`、`mbx test -p nola_cli --lib mac_launch_origin_contract_tests`、`mbx test -p nola_cli --lib permission_recovery_entrypoint_tests`，测试参数均为 `-- --test-threads=2`；使用独立临时 `NOLA_HOME` / `NOLA_DB_PATH` 与 16 MiB Rust 线程栈，共享 mbx 编译缓存并等待 Mac 构建锁。
- 实际结果：**27/27、2/2、18/18 全通过，均 exit 0**；恢复组 13.85 秒。日志 `/tmp/nola-final-verification.gMkJRJ/mac-origin-protocol-2.log`、`mac-origin-native-2.log`、`mac-origin-recovery-2.log`。
- 新回归经过真实 native decode/hydrate：scheduled resume 保留冻结 descriptor、没有新输入；仍拒绝恢复帧新增 input_origin，fresh 来源不匹配仍拒绝。原 Run 持久化冷恢复覆盖 interactive/scheduled 两种来源，篡改冻结来源被 exact binding 拒绝且 checkpoint 不变。此处不等同 Swift 真实 helper 进程的端到端结果，更不等同实际 provider 验收。
- 随后 `mbx check --workspace --all-targets` **exit 0**，35.15 秒；日志 `/tmp/nola-final-verification.gMkJRJ/workspace-final-origin-check.log`。测试期间未穿插修改生产源码。

### 2026-10-12 Mac 来源 wire 契约 Rust 定向验收

- 源码冻结后，使用 `mbx test -p nola_host_protocol --lib`、`mbx test -p nola_cli --lib mac_launch_origin_contract_tests`、`mbx test -p nola_cli --lib permission_recovery_entrypoint_tests`，均附 `-- --test-threads=2`；独立临时 `NOLA_HOME` / `NOLA_DB_PATH`，`RUST_MIN_STACK=16777216`，复用共享编译缓存。
- 三项分别 **27/27、1/1、17/17，通过，exit 0**。日志为 `/tmp/nola-final-verification.gMkJRJ/mac-origin-protocol-1.log`、`mac-origin-native-1.log`、`mac-origin-recovery-1.log`。验证闭域必填字段、fresh origin 匹配矩阵和既有原 Run 权限恢复，不是实际 provider 验收。
- 同批只读审计发现尚未覆盖的真实缺口：Swift resume 主动去除顶层 `input_origin`，同时保留 scheduled 来源的原 `mac_launch`；Rust 当前统一来源比较将缺省 InteractiveUser 与 ScheduledAction 对比，导致 scheduled cold resume 被拒绝。须在下一源码批区分新输入准入与 exact-Run 恢复，保留原冻结 binding 权威；本批通过不宣称此路径已验收。测试期间未修改生产源码。

### 2026-10-12 Mac 真实 Host 定向组合第一轮

- 在 iOS7 释放 guard 后运行；沿用上述隔离 Test DerivedData/XCTestDevices，未覆盖 Debug。`test_mac_layers.sh cancel-e2e` exit 65，2 项 / 2 unexpected failures；日志 `/tmp/nola-mac-contracts.SuNCkV/cancel-e2e-1.log`。
- 两项均被 `MacCancellationCLIEndToEndTests` 独立 fixture 缺模型 capability profile 挡在 begin 准入，尚未覆盖真实 cancel-only 进程。只读扫直接 begin fixture 另发现 `MacAssistantJournalIngestionTests.seedSource` 同类遗漏，该 suite 本轮未执行。
- 随后 guard 包装 `xcodebuild test -scheme NolaMac -configuration Test`，仅选择 `MacAgentCapabilityEndToEndTests` 的 WholeAppColdPermissionRecovery、ConversationPermissionIsReusedOnlyForMatchingBashScope、OncePermissionDoesNotPersist、ConversationPermissionDoesNotLeak 四项；exit 65，4 项 / 4 unexpected failures，2.433 秒；日志 `/tmp/nola-mac-contracts.SuNCkV/host-permission-1.log`。
- 三个 scope 用例在启动失败：Swift Mac launch descriptor 发送 `input_origin`，当前 Rust `HostMacLaunchDescriptor` 缺该字段且拒绝未知字段，是真实协议接线缺陷，不是模型授权拒绝。cold 用例快速返回 `CancellationError`，本轮证据不足以归因，不能宣称已覆盖 kill/recover 路径。
- 模型端为本地 mock server，不是真实 provider 验收。所有工作区、数据库、应用支持目录为 fixture 临时目录；杀进程代码只针对持有的具体 conversation/Run Host PID。完整失败收齐后未修改源码或重测，guard 已释放，CrashReporter 设置已恢复。

### 2026-10-12 Mac Host 定向组合收尾复验

- 第二轮：`cancel-e2e-2.log` exit 0，取消 2/2；`host-permission-2.log` exit 65，scope 3/3、ingestion 1/1，cold 唯一断言误把无 requirement assessment 的普通模型回答要求为 ActionCompleted。随后改为严格 Answered，副作用与身份检查独立保留。
- 第三轮：`host-helper-3.log` exit 65；12 项中 10 通过、2 cold 失败（4 assertions / 4 unexpected）。两种来源均已实际恢复并封存，但终局精确读取暴露 Application 仍按旧状态判定 Run 的生产缺陷；watchdog 清理还产生二次 CancellationError。未删除终局读取回归，下一冻结批修复 sealed 语义和显式 cancel+await watchdog 清理。
- 第四轮：`/tmp/nola-mac-contracts.SuNCkV/host-helper-4.log`，exit 0、`TEST SUCCEEDED`；12/12、0 skipped、0 failures，22.152 秒。
- 组合为 capability 5（scope 3、interactive/scheduled provenance cold 各 1）、cancel 2、ingestion 1、resume frame 4。guard 包装 `xcodebuild test -scheme NolaMac -configuration Test`，逐项 only-testing，沿用隔离 DerivedData/XCTestDevices；本轮 application/store/service/cli 实际重建 37.70 秒。
- cold 检查包括原权限请求、仅拥有的 Host PID 终止、错误 owner/workspace/credential 拒绝不变、原 Run/assistant terminal seal、真实文件内容及 readback、冻结 invocation digest 和来源不变、终局精确读取。scheduled 用例证明来源恢复，不宣称覆盖完整 automation occurrence/lease 调度。
- 本组模型端仍为 fixture server；真实 provider smoke 有独立记录，不混淆计数。未修改用户数据或签名 Debug，未穿插改源，结束后 global guard 释放且 CrashReporter 设置恢复。
# 2026-10-12 真实压缩复验失败记录

修正证据来源后的完整复测通过：`/tmp/nola-mac-real-memory.VmJLJd`，日志
`/tmp/nola-mac-compaction-canonical-gate.log`，runner exit 0，XCTest 1/1、零失败/跳过，
evidenceProfile=runtime。使用未变 Test 二进制和全新隔离数据库，构建 digest
`745e5fa1def15af91a9a70fee647458254a7115129304ddb1b148a3f8afd73dd`。
真实 128-token 摘要截断留下 315 字节，续写一次后 canonical 完成与计划来源匹配；
截断采样计为 incomplete（3405.94ms），严格 ACK、原文保留和 conversation_recall
均通过。不是自然耗尽模型窗口，也不是产品 UI 视觉/全端验收；既有失败样本仍保留。

schema 17 新构建真实样本：`/tmp/nola-mac-real-memory.SEJqYo`，外层日志
`/tmp/nola-mac-typed-recovery-real.log`。XCTest 1/1、零跳过/失败；原 runner exit 1，
因为尾部门禁读旧 trace_journal_events（实际为空），不是生产续写失败。
canonical run_events 保存 cached_request_fork 截断及 summary_continuation 接受，
部分正文 458 字节、continuation_count=1，随后 compaction_completed 来源指纹匹配。
provider 实际注入 128 输出 token，返回 max_output_tokens；measurement 正确记 incomplete。
同一 XCTest 已完成严格 ACK 铺垫、压缩、conversation_recall 及原文来源检查。

门禁改用 compaction_continuation_evidence.py，只读当前 run_events，不回退旧 Trace。
按同 Run 的计划→同 effect/turn/request digest 的不完整/续写→来源指纹匹配的完成
关联验证；不冒充完整 Host fencing 或 terminal seal 验证。
`python3 -m unittest discover -s scripts -p test_compaction_continuation_evidence.py`
2 测试（含 8 拒绝子例）通过；在上述真实库离线核验返回 continuation_count=1、
partial_summary_bytes=458。修正门禁后的完整 runner 尚未重跑，不能将原 exit 1 改称 0。

单样本阶段值：截断热分叉 2459ms、cache read 20315/20988；续写 4458ms、
21115/21169；后续一次热分叉 4807ms、21191/21893；一次冷增量摘要 3121ms、
0/4594。它们是 Host 请求阶段值，不是用户端总压缩延迟或 p95/SLO。

恢复状态类型化复测：`/tmp/nola-typed-recovery-state.log`，命令
`nola_cargo test -p nola_core --lib --test agent_core_host_contract`，Core 库 486、
Host 契约 142 全通过，exit 0。当前 checkpoint schema 17；覆盖 enum 往返、
缺字段/缺原因解码拒绝、空原因验证及既有冷恢复行为。不证明外部客户端二进制
已重建，不改变用户数据库；真实 Mac 需要新 Test 构建和隔离新会话复验。

恢复出口完整分支增量：`/tmp/nola-loop-recovery-exits-final.log`，命令仍为
`nola_cargo test -p nola_core --lib --test agent_core_host_contract`，485+142 通过，
0 失败/忽略，exit 0。新增非法 QuestionSet 退出、直接回答不强制提问、重复/空/
无效控制/非法问题四分支的 context input usage 精确增量断言。首次新断言
Option<usize>/Option<u64> 类型不匹配导致编译失败，显式 checked 转换后复测通过。
无真实 provider 调用，不覆盖持久 usage ledger 或 UI/seal。

恢复出口补测：`nola_cargo test -p nola_core --lib --test agent_core_host_contract`，
`/tmp/nola-loop-control-exit.log`，485+142 通过、0 失败/忽略、exit 0。新增冷恢复
无效 requirements_update 不生成后继模型请求、不产生用户等待。生产出口之前
移动 context usage 记录；尚无针对该分支的 usage 数值断言，不宣称计量完整性已验收。

后续权限失败核对及最终复测：旧用例将两个不同文件视为同一授权，并期待 Host
grant.resource_pattern="*" 扩权，违反当前 Core 具体资源契约。改为验证第二个
请求仍挂起、单独批准才派发及独立回执；未放宽生产授权。命令同下，最终日志
`/tmp/nola-loop-permission-contract-final.log`，Core 库 485 通过、Host 契约 142
通过，0 失败/忽略，exit 0。覆盖模块边界，不代表真实 Mac/跨端验收或性能达标。

循环恢复增量回归：`nola_cargo test -p nola_core --lib --test agent_core_host_contract`，
日志 `/tmp/nola-loop-recovery-contract-final.log`。Core 库 485/485 通过；Host 契约
141 通过、1 失败，整体 exit 101。新增冷恢复 no_progress 路径及显式用户等待续行
用例通过。失败项为
`allow_conversation_resolves_matching_permission_requests_already_pending_in_the_same_batch`，
执行数实际 1、预期 2，尚需核对范围语义及生产消费。不得声明整套验收通过。
此前一次库测试仍断言旧的强制提问提示，已按新语义修正；生产真实 Mac 尚未复验。

新构建复验：`/tmp/nola-mac-real-memory.b1uzdR`，日志
`/tmp/nola-mac-answer-contract-compaction-real.log`，同参数但重新 build-for-testing，
exit 65、0 通过/1 失败/0 跳过。失败为 MacLocalAgentWaitSuspension，首轮 ACK
发生重复召回。修正后的事件分类不再误报 Trace；本次没有摘要截断注入命中，
不得作为压缩续写证据。已终止，无运行中的测试进程需要等待。

随后控制引导边界整改的模块回归：`nola_cargo test -p nola_core --lib
agent::runtime::requirements::tests`，最终日志
`/tmp/nola-requirement-answer-contract-final.log`，7 通过、0 失败/忽略、478 未选中，
exit 0。覆盖无工具 observation 的显式 answered、范围/水位及控制事务回滚；
不证明模型会遵循引导，不替代真实压缩验收。首次编译因新测试误用
pending_effects.is_empty 失败，修为既有 iter 接口后得到此最终结果。

`NOLA_SKIP_MAC_MEMORY_BUILD=1 NOLA_MAC_REAL_MEASUREMENT=1
NOLA_MAC_REAL_WIRE_PROTOCOL=responses NOLA_REAL_COMPACTION_WINDOW=40000
NOLA_MAC_REAL_COMPACTION_TRUNCATE_TOKENS=128 ./scripts/test_real_mac_compaction.sh`
（另指定既有 Test DerivedData 与保留产物）。产物
`/tmp/nola-mac-real-memory.jv08dS`；exit 65，0/1 通过、0 跳过。
未实际触发摘要截断。隔离数据库记录 ACK 铺垫轮重复历史检索及随后用户决策等待；
XCTest 将 userInputRequested 错归为必须附 Trace 的事件。辅助函数分类及每轮
严格 ACK 前置条件已改，未重跑。不得将 HTTP 200/completed 当作压缩场景通过。

