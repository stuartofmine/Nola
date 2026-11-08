> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

# Goal、子任务与远程执行验收清单

更新：2026-10-28。此表记录当前版本的剩余工作；各审计中的早期失败保留历史，但不能覆盖后续明确证据。总体未完成，不能以一个平台或一个子任务替代全范围验收。

| 范围 | 当前证据 | 仍需完成 |
| --- | --- | --- |
| Web Goal 控制 | 当前 epoch8 真模型两轮、暂停丢回执、重载恢复、取消及运行历史通过 | 多专家/提问/故障组合不由单 Goal 验收替代 |
| Web 子任务 | 当前 live 子任务只读进入、助手结果391、刷新、返回父会话并实际停止通过；唯一子终态回执 | 多任务指导与网络故障组合 |
| iOS 远程结果 | 已有 paired 父子结果与导航证据，适用构建详见下文 | 当前完整长任务、指导与取消故障矩阵 |
| iOS 本地子任务 | 原 Requires 审批冷恢复、65秒等待后实际批准、精确391结果与原父子 Run 封存通过；cancel_requested=0 | 多专家并发/网络故障的扩展矩阵不由单链替代 |
| 原生 Goal 操作 | Core 统一控制能力；Mac 同原 Goal 暂停丢回执、继续、取消、冷开真实通过；移动端合同接线通过 | iOS/Android 同等真实故障组合；不能以 Mac 代替 |
| Android 真实验收 | 原取消子任务同 Run 封存且无新模型尝试；原冷恢复任务最终准确剪贴板结果、父子 answered、唯一终态回执，未误取消 | 远程 Goal/双专家故障组合；原工具计划过期失败保留为同 Run 重试前的失败 |
| Mac 原生验收 | 9项来源/能力合同和真实 Goal 窗口控制通过；原子任务冷开源记录/镜像摘要一致、任务卡AX及sealed终态验证2/2通过 | 真实子任务指导丢回应；系统遮挡下未完成整窗视觉验收 |
| 取消证据 | Web 实际 Stop；Android 原取消任务唯一 canonical terminal receipt，attempt和模型尝试未增加 | 多子任务取消与网络丢回应；未知外部效果不得冒充已停止 |
| 交互与全端回归 | Web 536项回归通过；各端局部真实按钮、导航、草稿、结果证据保留 | 当前构建完整长任务切换、键盘/遮挡、权限与终态矩阵 |

移动端原任务的过期审批恢复与结果已完成真实验证。Mac首次提交已实际创建并完成子任务；冷开同步的 `schemaVersion` 编码问题已修，同原Run源记录/镜像摘要核对与原任务卡AX验证已通过。共享Core另补并发迟到chunk的精确重放确认，Store全量458/0/4通过。真实指导丢响应仍未完成。下文为按时间追加的审计记录，早期“尚未实现”不代表当前实现状态。

## 当前整批修改与统一验收门槛

用户要求先完成整批改动，再编译测试。首批已完成统一验证，真实运行发现的问题继续按批次修复。Core完整五包 r6 的两个失败已分别由 Store lib 453/453、Runtime Driver 15/15收口，原失败日志保留。iOS原生77项及真实paired父子结果导航通过；Android原父子会话在客户端与Host双冷恢复后结果导航通过。Chrome新Goal及原失败Goal的真实两轮、暂停丢回执恢复、取消均通过。当前新增批次包括正式运行记录收敛、Mac持久来源归属、共享本地delegation执行链；此批不能用此前构建的结果替代验收。各证据路径与适用范围见文末。

| 本批合同 | 修改状态 | 统一验证必须证明 |
| --- | --- | --- |
| Goal 固定执行绑定 | Core 原子 Start、CLI/Web 冻结恢复、Mac 私有 MLX channel 重新准入已编辑 | 首 Run 前崩溃恢复不改模型；撤销配置阻止执行但允许取消；MLX 冷开仍使用原 logical model/revision |
| Goal 固定操作身份 | Web/Apple/Android outbox 与 Core receipt 已编辑 | 丢响应、重载、原操作重试只产生一次业务变化；拒绝与未知状态区分；Host/会话隔离 |
| 同 Run 指导后消息分段 | Core Segment/Effect/Activity 所有权及终态正在收尾 | 已应用的指导后新输出排在指导后；既有卡片晚更新不漂移；空新段取消可封存；完整多段冷恢复 |
| 子任务取消 | 正在统一 Core 与各端 exact parent owner 合同 | 父 Run 结束后后台 child 仍可取消；不误投递到后继 Run；请求已持久化不等于任务已停止 |
| 远程观察身份 | Host/Web 已补顶层会话及 client，移动端核对中 | result=null 也不能解锁其他会话/client；断线保持未确认 |
| 移动端本地子任务 | 共享worker已有CLI调用；有界mailbox/FFI及原生父链、独立child executor正在接线，未统一验证 | 不能把 paired CLI Host 的真实执行计为移动本地支持；完整实现仍保留在总目标范围 |

统一验证顺序：共享 Core/CLI 合同和真实 SQLite回归 → Web 类型/组件与原生编译回归 → 使用当前 epoch8 隔离 Host → 浏览器及模拟器真实按钮、模型、长任务和故障恢复。保留旧隔离故障库，不升级或重建用户数据。真实任务至少覆盖 Start/Pause/Resume/Cancel、双子任务指导与取消、退出重开、任务详情返回、权限等待、原请求丢回包重试。

## 本轮确定的实现缺口

- Android `ConversationProgressPanel` 对远程Host直接拒绝，缺少paired progress reader；不能用本地镜像冒充Host权威。正在添加只读薄桥，复用Core Goal与plan投影。
- iOS `DeviceAgentCoordinator.sendTaskGuidance` 使用已删除的 `queue_task_guidance` AgentSubmission。改接当前typed Application指令与持久回执，不能保留旧字段/接口兼容。
- Mac `task_guidance` 是当前desktop控制frame，不等同于上述旧Core命令；其缺少固定消息身份和回执恢复须单独修复。

证据标准：编译、组件/协议fixture、真实SQLite、真实OS、真实模拟器/浏览器、真实模型分别记录。按钮返回202不算控制完成，必须核对Core回执/终态。缺少任一范围的证据即未验收。不得清库、伪造回执或将旧数据转换来消除失败。

## 当前轮次进展与真实失败

- 已添加 paired `/api/mobile/chat/agent/v2/progress` 只读薄桥，直接读取既有Core Goal与conversation plan；只读请求核对Host和conversation，未知会话明确拒绝。定向Core/Host测试1项通过，真实未认证HTTP返回401。Android reader/进度面板已接，尚待当前包真实非空Goal验证。
- Android真实配对成功，提交在工具计划时间检查分支失败，没有模型Run。检查发现四端客户端重复用本机时钟判断Host签发计划新鲜度；现统一按Core结构/身份语义解码，执行Host仍在准入时严格校验过期和目录revision。Web5项与check通过、Android工具计划+进度8项通过；Apple待统一native测试。没有捕获首次请求完整时间差，不声称已实测具体偏移。
- iOS已移除旧guidance命令并使用现有持久outbox/typed admission与receipt；保留task_control的支持能力字段。取消AgentSubmission仍是有效当前合同，仅按Core能力门禁，未错误删除。新增native相关12项通过；整组35/43中8项旧测试桩缺worker identity命令响应，桩已更新待复验。
- 5157真实双子任务：A、B均准入且answered，A指导delivered；B取消未及时执行，父Run仍running，不计通过。定位父长模型batch阻挡CancelTask dispatch，126秒后才claim已超过30秒deadline；过期ExecutionRejected在Core无call_id控制失败路径返回Err并回滚。正在共享runtime-driver/Application修控制面调度及失败持久化，不加Web旁路；原失败库保留。

- 共享控制修复定向结果：runtime-driver 12项、实际SQLite selective claim/full fence/过期控制settle 1项、完成先于晚取消1项通过，CLI构建成功。原5157正常重启后父Run终结为failed（旧模型请求超时），两个child保持answered；这是恢复到明确失败，不是多任务汇总成功，也未重发取消/消息。5159新双任务正在真实重验。
- 工具计划Host时效保留已用真实Store验证：`prepared_plan_survives_reopen_without_extending_expiry_or_owner` 1/1通过，冷开不延长过期边界且foreign owner拒绝；见 `/tmp/nola-host-plan-expiry-final-test.log`。

## 最新验收与新增缺口

- 5159真实双任务：父Run `110f3a68…` answered，A answered，B cancelled，唯一指导delivered。取消claim约9秒，未再超过30秒deadline。原脚本错误地要求父汇总包含仅要求子A输出的marker而超时；保留原失败报告，按子任务权威结果与父汇总分别验证，最终冷恢复UI仍待完成。不能把此脚本超时记为产品取消失败，也不能跳过最后UI断言。
- Web当前完整自动化521/521通过（`/tmp/nola-current-goal-web-suite.log`）；这是自动化回归范围，不代表原生或全部真实故障验收。
- Android当前原生构建成功、新包安装冷启动成功。真实新会话又发现“Use Host”仅修改进程内ModelService，冷开丢失选择并在发送时出现No model configured；无模型的本次请求没有Host Run。必须补选择持久化和无模型发送入口引导，不能以手动重选代替修复。
- iOS `IOSConversationInspector` 仍固定从本地runtime读取会话投影/plan，没有paired Host来源分支。需要复用现有paired progress薄桥，严格核对source Host与conversation；远程进度不能由本地镜像代替。此项与原生Goal控制缺口分别验收。

## 跨端本轮闭环（2026-10-26 后续）

- 5159最终真实UI通过：点击A子会话读取完整journal确认指导marker，返回父会话Stop消失、输入草稿后Send可用。最初测试错误地在父汇总找子marker，以及未等待异步task详情渲染，原失败报告保留；未改产品结果绕过断言。发现同Run指导气泡与累积assistant回答的历史排序仍需后续处理。
- iOS当前指导/能力/时钟native集中44/44通过；paired progress新读取已落到精确Host来源。真实iOS源plan已生成并可见，冷恢复限定sheet内文本的最终复验正在进行。
- Android真实父子任务各answered；Host选择→退出→新会话冷恢复通过。Web同原会话创建/暂停Goal→Android冷开刷新objective、paused、pending要求一致，证据 `/tmp/nola-android-shared-goal-final-proof/report.json` 与 `/tmp/nola-android-remote-progress-evidence/goal-cold.png`。该截图仍暴露内部BuildingContext混排，Android后续UI改动已移除，待统一新包真实验证。
- 新paired Goal薄桥 `/api/mobile/chat/agent/v2/goal` 已实现，传输解密/Host身份核对后与Web复用同一 `dispatch_conversation_goal_control`；不直接接收Core命令、不由客户端改Goal投影。真实SQLite start/pause/旧token拒绝/resume/cancel/重新读取投影1/1通过；现有真实fixture HTTP长Run/审批取消2/2通过。尚未宣称原生控件已接入或真实网络丢响应恢复完成。
- `model_intent`是每轮冻结配置，不能自动当作永久会话选择。Android源会话直接Web启动Goal曾明确拒绝“没有保存模型”；通过真实Web模型菜单显式保存后Start/Pause成功。原生Goal启动尚需明确模型选择合同，不通过静默写用户偏好规避。模型选择首个测试误等configuration PATCH（实际是conversation PATCH）超时，脚本已修，原报告保留。
- Core终态投影已清除过期current_step，保留原观测progress且不制造100%。真实Store终态整组12/13通过；剩余用例暴露新CancelTask调度在source/global Store分离时读取错误任务库，正在按真实跨Store授权修复，不能以更新fixture掩盖。

### Paired Goal 控制当前接入合同

`POST /api/mobile/chat/agent/v2/goal` 使用现有paired bearer及加密JSON，不接受任意Application命令。请求为：

```json
{"expected_host_id":"source-host","conversation_id":"conversation","control":{"operationId":"client-generated-stable-uuid","action":"pause","expectedGoalId":"goal","expectedControlToken":"Core返回的opaque token"}}
```

`control`与Web使用相同严格DTO：`start`带`objective`及可选`constraints`，其他操作使用当前Goal身份/control token；支持`pause`、`resume`、`cancel`及已有`schedule`。时间由Host提供。响应为Web现有`conversationId`/`goalProjection`/`operationId`/`authoritative`/可选`controlPending`字段加`source_host_id`。HTTP202只代表待处理，必须继续读取Core applied_operations，不能直接显示已暂停或已取消。来源不匹配在任何变更前拒绝。创建仍要求明确保存的会话模型，不静默套用客户端或Host当前默认。

当前入口复用已有操作生成逻辑，尚未提供客户端固定intent重试合同；响应丢失时只能读取权威状态，不能自动重发新start/control。原生控件接入与持久intent/receipt收敛仍列为未完成。

- Android跨端进度验收用Goal已由真实Web“取消目标→确认”收尾，Core状态cancelled/revision3；未删除会话、原父子结果或恢复模型执行。证据 `/tmp/nola-android-shared-goal-cancel-final-proof/report.json`，同时验证已选模型菜单不会重复保存。此前脚本未处理选择项的✓前缀，原失败报告保留并修正定位。
- source/global分库取消确认是生产授权缺口：source outbox能证明控制，但global task不在source库；不能将“unknown task”吞掉，也不能由CLI直接写全局取消。将沿用existing admission trusted IPC的导出/导入方式，传递exact request/fence/admission；返回持久cancel_requested而非changed布尔值。当前修复尚未接入，不计通过。

### 当前产物真实传输补验

- 新CLI构建通过 `/tmp/nola-paired-goal-control-final-build.log`。独立5160 Host（不替换5158原生验收Host）经Chrome新建会话、显式选模型、Start/Pause通过，报告 `/tmp/nola-paired-goal-ui-preparation/report.json`。
- 实际AES-GCM加密HTTP共7次请求通过 `/tmp/nola-paired-goal-http-report.json`：匿名Goal请求401；progress为真实paused；foreign Host取消409且投影未变；旧control token取消409并持久拒绝回执；合法取消200/applied_operations无rejection；最后progress与取消回执完全一致。测试脚本 `web/playwright_runtime/real_paired_goal_http.py` 无mock，不打印凭据。此项证明实际paired传输与Core共享控制，不代表原生按钮或丢响应重试已完成。
- iOS最终 `/tmp/nola-ios-progress-cold-final.xcresult` 7/7 native加1/1真实冷启动UI通过：原source会话Progress内两个实际步骤可点击命中，8次跨刷新采样无spinner/内容消失，未新发模型。Mac共享guidance/outbox5/5及remote clock1/1通过；Mac完整Test编译成功。

### 分库取消与终态后续结果

- 分库取消已落到source exact started delivery授权导出→global trusted import，Core真实in-memory source+独立SQLite任务库测试通过；普通JSON不能构造trusted import。same-store和transfer都返回持久cancel_requested，不再误用UPDATE changed布尔值。新service入口还补“取得锁后刷新验证时间”，避免排队后使用过期前时刻；其定向测试由Mac代理收尾。
- `/tmp/nola-task-terminal-step-final-test.log` 当前13/13通过，包括父取消→真实global任务取消意图→worker取消收据→晚导入父且原seal保持不变、重复导入/重开、错误所有权与事务回滚。终态current_step清空，原progress不被改为100%。首次新fixture沿用取消后failed finish，被Store正确拒绝，后修为执行方cancelled收据，不弱化生产规则。
- 原生Goal绑定核查发现更深实现缺口：Apple尚无共享GoalRuntimePort执行桥；Goal Start自身尚未冻结完整模型执行绑定，冷唤醒可能重新读取会话设置。正在设计Goal专属持久执行绑定与准入，复用现有运行描述符/凭据引用；不能用客户端循环或永久保存普通每轮model_intent来冒充实现。

### 当前实施与新增真实发现

- 服务等待锁期间取消授权过期的定向回归1/1通过，`/tmp/nola-split-control-service-deadline.log`。该项验证真实mutex排队及执行时钟；分库授权由独立SQLite测试覆盖，不冒称真实模型测试。
- Goal专属绑定正在落入Core，Start与绑定同事务，复用既有Host vault引用。当前单版本Store epoch调整为7；不迁移、不重建用户库，旧测试Host继续使用其原进程和隔离库。新入口/Host接线及回归尚未完成。
- Android最终APK构建通过，但只读子任务真实测试发现：冷开父会话没有恢复task订阅，消息存在而任务卡缺失。失败证据保留于`/tmp/nola-android-existing-child-detail-real.log`；正在复用现有owner任务订阅修复，尚未标记子任务详情验收通过。

### 固定操作身份接线（编辑中，尚未统一验证）

- Goal control现要求客户端operationId。Host先通过既有Application command receipt准备并保存原始envelope/首次Host时间；同一operationId不同payload拒绝，重发同一请求读取原操作，不再生成替代Goal。复用现有回执表，不新增并行状态库。
- 非执行控制从已保存Core template提交；模型/凭据撤销不再阻止用户取消Goal。执行准备与每轮准入仍独立验证当前权限，已补禁用模型后取消的用例编辑。
- 共享创建Goal/requirements helper下沉agent-engine，runtime-driver及Application复用同一实现。Web与Apple/Android持久intent和原生控件接线仍在进行。
- 遵循用户“全部改完再编译测试”：本节所有新改动均为待验证，不以旧构建通过替代新产物证据。

### 移动端本地执行范围纠正

- Android与iOS embedded Host目前均无完整spawn/delegation executor。Android此前真实父子任务验证通过的是paired CLI Host，不能计为Android本地子任务执行通过。
- 共享Core已有子任务准入、lease与终态收据，但实际子任务runner仍依赖CLI HostInvocationRequest/AgentRegistry及进程驱动；移动ABI只有Application命令桥，不能用客户端循环替代缺失的共享executor。
- iOS本地cancel_task发现合成cancel_requested成功回包；正在改接exact Core取消回执，缺执行能力明确失败，绝不将取消意图显示为真实停止。
- Web新增Host隔离、跨标签页锁及原请求恢复用例；Apple/Android远程Goal本批已编辑完成，Mac完整入口仍编辑中。本批尚未统一编译/测试。

### 本批静态收尾补充（未运行验证）

- Web Goal 面板补齐 Host 切换响应依赖、跨标签页 intent 通知、重试后 pending 状态同步、权威拒绝提示；刷新后可从原 Start intent 恢复目标文字。网络失败后的回读也有明确超时，避免永久占用交互状态。
- 真实浏览器丢回包脚本新增“重载后点击重试原操作”步骤，要求再次发送的完整 payload 与首次一致，再由真实 Host ledger 确认。脚本尚未运行，不能计入真实测试通过数。
- Core 多段回答终态审查发现空新段立即取消的 frontier 边界：当前全 Run 无 effect 才允许空段的判断不足，交分段实现一并修复；不能用虚构文本规避封存约束。
- Apple summary 已改为消费封存清单的所有 assistant 段，并补 admission/head 读取竞态修复及测试草稿；仍待统一编译与真实验证。

### 整批统一验证第一轮

- 本批代码收齐后启动统一验证。Web `npm run check` 0 errors / 0 warnings（`/tmp/nola-goal-batch-web-check.log`）。首次完整suite因新增构造函数参数属性不受Node strip-only支持而失败；改为显式字段赋值，不改变控制语义。受影响三文件28/28通过，再次完整`npm test`532/532通过（`/tmp/nola-goal-batch-web-tests-final.log`）。这是自动化协议/组件证据，尚非真实模型与按钮验证。
- 共享Rust首次编译未通过：分段projection语法误插及Goal owner类型路径、Repository trait bound遗漏。已分配集中修复，首次失败日志保留`/tmp/nola-goal-batch-core-validation.log`，不得视作Core通过。
- 当前iOS booted测试模拟器为28F93564-C782-466C-9684-70C2669BED20，Android emulator-5554在线。此项仅设备准备证据，尚未执行本批真实任务。

- Web生产构建和交互测试资源构建通过，日志`/tmp/nola-goal-batch-web-build.log`、`/tmp/nola-goal-batch-web-interaction-build.log`。生产bundle仍有体积提示，不是构建失败。
- 真实Chrome组件点击测试通过`/tmp/nola-goal-batch-task-clicks-r2.log`：指导、错误归属回执、未确认取消、重载防重复、终态控件。首次失败因测试拦截器未覆盖新嵌套`cancel/receipt`路径，修正拦截范围后复验。该测试使用明确合成回执，不计真实模型/真实任务Store验收；专用Vite进程已停止。

### 统一验证暴露的生产缺陷（尚待复验）

- 新分段真实SQLite用例首次失败（`/tmp/nola-segment-prebuilt-test.log`）：首次Run准入在写入初始Core事件前验证effect归属，导致合法声明找不到。修复将精确事实验证放在初始事件写入之后，仍在同一事务内；下轮要求准入失败回滚用例一并通过，不能放宽归属规则。
- CLI Goal绑定首轮2通过/1失败（`/tmp/nola-goal-cli-binding-validation.log`）：冷恢复遗漏既有Host memory admission恢复步骤。按冻结Goal template恢复原memory policy并维持digest校验，属于生产修复，不是通过修改fixture绕过准入。
- Core旧port测试编译发现Goal依赖过宽传播至普通journal publication。正在收窄Goal准入bound并提取终态只读接口；保留内存port的完整事务快照、no-op和fail_commit断言，不能为通过编译加入未实现的repository桩。

- CLI Goal绑定修复复测3/3通过（`/tmp/nola-goal-cli-binding-final.log`）：首Run前持久绑定及冷恢复不改默认模型、固定控制意图重试、凭据撤销后阻止执行但允许idle暂停。该结果验证本次memory admission恢复修复，不等于原生MLX真实冷恢复通过。
- 当前runtime-driver单元产物6/6通过（`/tmp/nola-batch-runtime-driver-prebuilt-tests.log`）。Domain完整旧本轮产物234/235，唯一valid projection fixture漏当前execution_ownership字段已补，等待统一重编；不记为235通过。

- 已编当前CLI产物新增原生控制验证：native Goal channel 1/1、固定Goal control port 1/1、desktop_host_control 10/10通过，日志分别`/tmp/nola-goal-native-channel-test.log`、`/tmp/nola-goal-native-control-wire.log`、`/tmp/nola-goal-desktop-controls.log`。仅合同/线程传输测试，不等于MLX真实推理验收。
- Mac Test已启动，复用`/tmp/nola-a2ui-mac-derived`的Products/Test。scheme的NOLA_TEST_MODE=1使Store位于每轮独立`Nola-XCTest-<PID>-<UUID>/ApplicationSupport/Nola`，不改现有用户Store。结果仍等待，不能计为通过。

### 统一验证：准入顺序与跨库发布（2026-10-26）

- 真实 SQLite 定向 39/39 通过：准入与回滚 10 项、Goal 执行绑定 6 项、任务 Run 绑定及指导分段 23 项，见 `/tmp/nola-segment-admission-focused-r1.log`。生产事务顺序为基础 Admission/Part 所有权 → 初始 Core 事件 → 完整 Effect/Activity 所有权校验，保持同事务回滚。
- Agent Engine 全部 564 项通过；Store lib 为 447 通过、6 失败、4 忽略，见 `/tmp/nola-segment-engine-store-lib-r1.log`。先前十项用户提问 provenance 失败随准入顺序修复消失；剩余当前协议 fixture 正在修正，尚未宣称 Store 全通过。
- Application lib 为 357 通过、5 失败、1 忽略；local projection window 2/2，见 `/tmp/nola-segment-projection-focused-r1.log`。五项失败揭示实际跨库缺陷：trusted receiver 启动 child 时无条件向不属于本 Store 的 parent journal 发布。已按可信 admission 的 source_store_epoch 限定发布所有权；同源缺失 owner 仍拒绝，接收端不伪造父投影。新增 source/receiver 对照回归，待新构建验证。
- iOS 七组 native 测试已启动并复用现有派生目录；CLI 当前 epoch 7 增量构建进行中。新 Host、真实 Chrome 与模拟器故障验证尚未运行；旧 epoch 6 证据不能替代本轮结果。

- 跨库修复定向复验 14/14 通过、1 个子进程专用入口忽略：`/tmp/nola-segment-cross-store-wait-r1.log`。覆盖 receiver 无父 journal 的合法启动、同源缺所有权拒绝、用户回答与取消竞争、等待状态冷恢复；没有改变原五个跨库 fixture 来回避生产问题。

### Epoch 7 真实执行发现（未通过）

- 新隔离 Host 5161 使用当前 CLI 和 epoch 7 Store。真实 Chrome Goal Start 已收到模型完成；暂停丢响应后重载、固定原操作重试及原 Run 封存均成功。但 Resume 返回 running 后 120 秒无新 Run，报告 `/tmp/nola-epoch7-goal-real/report.json`。Host 证据显示第二轮被 `Recovery Host catalog changed` 拒绝，正在定位冻结配置与重建 catalog 差异，不能记为 Goal 全闭环通过。
- 实际双子任务已启动，其中一个取消后 sealed；另一项仍在运行。并发 native 构建一度将磁盘压至约 200 MiB，Host 出现 SQLite unable to open database file，需要保留并核对实际失败边界。仅清理旧的可再生 Rust 测试执行文件约 5 GiB；保留所有源码、当前 Host、日志、故障数据库和用户数据。可用盘恢复到约 5.3 GiB。

### 原 epoch 7 双任务恢复后的真实 UI

- 原双任务在不重发、不改库的情况下自然达到父 answered、A answered、B cancelled。只读证据 `/tmp/nola-epoch7-child-recovery-readonly.json`；旧 effect 的租约到期后重新准入 successor，不能因暂时失联推断已停止。
- `/tmp/nola-epoch7-recovered-ui-r3/report.json` 真实 Chrome 重开原父会话→展开任务→点击子会话→看到指导 marker `MULTI_A_VERIFIED`→返回父会话→确认 125250 与取消说明→输入草稿且 Send 可用，全部通过，未调用新模型。首次脚本未展开外层 details、第二次子页加载超过 30 秒均保留报告；第三次没有延长超时且成功，不能据此宣称加载性能稳定。网络记录均为实际请求。
- 磁盘再次清理仅涉及未使用的旧 Xcode Intermediates/Index 与 Rust target/check，释放 12.07 GiB、可用空间恢复 17.22 GiB；当前原生输出、签名应用、测试日志、故障库与模型配置均保留。

- 磁盘恢复后的同一真实父/子历史四次冷页面读取分别为 419、256、302、278 ms（直到实际结果文本可见），见 `/tmp/nola-epoch7-load-timing.json`。未改超时、未改协议、未重新生成模型结果；暂不为单次并发构建期间的慢加载引入新的客户端缓存或状态层。这四次不能证明首次慢加载的唯一原因。

### 完整五包回归 r6 与 iOS epoch 7 收口

- `/tmp/nola-goal-batch-core-validation-r6.log` 完整五包运行结束：161 个测试目标，2368 通过、2 失败、7 忽略。剩余失败为短租约 heartbeat fixture 的有副作用时钟，以及 Store historical handoff fixture 不完整的 checkpoint metadata；均保留失败，待定向修复验证，不能记全绿。
- iOS 本批原生 77 项均已通过（r5 的72项 + summary-r6 的5项），额外 canonical source fixture 5 项通过。真实 UI `/tmp/nola-ios-epoch7-remote.xcresult` 1/1：新配对、冷启动、新会话、真正 read_only child 391、只读子结果页、返回父输入。父 Run `40f0c883-7990-44cd-b654-64084d86a60d` 和子 Run `8df8da00-c94c-41c8-8478-48da93555621` 均 sealed，使用本轮隔离 epoch7 库，无旧结果替代。
- Android 旧 JNI/APK 早于本批，正在单 arm64-v8a 重建；已新增结果页点击/返回与原会话冷启动用例，新 emulator user13 保留所有旧用户与数据，尚未执行新包验收。

### Core 最终合并证据

- r6 的两个失败均已收口：当前 Store lib 完整 453/453、4 ignored（`/tmp/nola-store-lib-final-current.log`）；Runtime Driver committed runtime 完整 15/15（`/tmp/nola-runtime-heartbeat-fixed-full.log`）。原 r6 退出101保留，合并证据不存在未解决的本批 Core 测试失败，不称原r6全绿。
- Goal CLI 新增正式 RunResult 传播，正常及冷恢复从精确 terminal seal 取得结果；blocked 不再由 completed Turn 推断成部分完成并循环重试。两个映射/缺失结果拒绝测试通过；真实预算阻塞仅一轮 NeedsUser 回归通过。第二轮HTTP恢复仍在查验，不能计Goal全闭环通过。

### Goal 冻结配置与真实两轮执行收口

- CLI worker 四项真实 HTTP 回归通过（`/tmp/nola-goal-worker-final-r10.log`），绑定三项及 canonical result 两项通过。第二轮 fixture 曾被单线程长连接占住 accept，改为每连接独立处理并统一关闭；保留原失败证据，不以延长超时替代验证。
- 新 CLI 对原 epoch7 测试库正常重启后，Chrome 完整点击通过：新会话 Start → 真模型第一轮 → 故意丢 Pause 回执 → 页面重载与原操作重试 → 同会话 Resume 真模型第二轮 → Cancel 回执202后权威确认 → 精确 Run sealed。`/tmp/nola-epoch7-goal-fixed-real/report.json`，会话 `conv-20261026225517470-1`，两 Run `goal-run-b5ef49a9-b01d-4e6e-aef5-a05582e1f6e3`、`goal-run-af9d8b7e-0b88-4fa2-984c-734d098f1a71`，无 pageerror。这是新 Goal 在原库上的完整验证；此前失败 Goal 的原绑定冷恢复另行验证中。
- Android 新 JNI 的真实父子任务已完成，但冷历史缺子结果入口：UI读取未写入旧meta导致任务关联消失。改用canonical Part.Task标识关联权威TaskProjection，保持结果读取所有权校验；两项映射回归通过，原会话冷点击仍在验证，不能先计通过。

- 原失败 Goal 也已真实 Chrome 恢复通过（`/tmp/nola-epoch7-goal-original-recovery/report.json`）：沿用 `conv-20261026212125787-1` 的原绑定和数据库，未重建 Goal；继续真实模型执行、暂停丢回执后原操作重试、再次继续及取消均确认，报告 passed=true。
- Android 原会话在客户端 force-stop 与 Host 重启后真实冷恢复 1/1 通过（73.606秒，`/tmp/nola-android-epoch7-cold-ui-r2.log`）：父答 marker → canonical task 卡 → 子结果391 → 返回父输入。仅Kotlin映射修复，原父子均已sealed，无重发任务。首次live流程因模拟器60秒自动息屏失败仍保留，不将其改写为完整live测试通过。
- 截图发现后续问题：Goal已取消但运行记录保留取消前running快照；并发读取Goal/traces后轮询仅根据Goal终态停止，漏掉后续trace收敛。上下文trace也被计为正式运行，需按权威归属分离，不能按ID前缀猜类型。此项待修，整体目标未完成。

### 运行记录收敛与共享执行权限（待最终真实 UI 复验）

- Core运行列表改按真实Run、conversation和root assistant归属筛选；保留诊断journal和按trace读取，wire不变、不造终态。真实Store冷开对照1/1通过（`/tmp/nola-trace-owner-current.log`）。
- Web不再因Goal终态而停止观察尚未终结的正式trace。活动面板10/10通过，svelte-check零错误零警告，构建通过；真实点击脚本新增无需手动刷新即可收敛运行记录的断言。等待新CLI与完整本批改动合并后执行，不计真实UI已通过。
- Task grant DTO单一定义收至Domain，运行权限、续租校验、权限挂起/恢复状态收至Runtime Driver，CLI真实调用删除重复实现。共享权限3/3通过（`/tmp/nola-shared-task-authority.log`），CLI实际Store调用回归待统一编译；移动本地executor仍未完成。
- Mac Test r3为44/45，唯一权限恢复失败揭示持久来源读取缺口：mirror无本地session projection，完成后提交intent清除导致Host归属无法查到。正在补共享只读来源归属并接Mac，保留失败日志（`/tmp/nola-mac-goal-task-contracts-r3.log`）。

### 共享本地 delegation 本批实现边界（未编译）

- 运行模型capabilities纯DTO移至Domain；profile路由、generation override、reasoning与capabilities解析移至Application `model_profile_resolution`。CLI resolver保留exact endpoint凭据获取及Host配置适配；RuntimeConfig/HostRuntimeOptions generation override同样调用共享解析，未保留第二算法。新增Core直接配置/transport校验测试，待整批统一运行。
- Mac持久来源归属窄查询已接入Core/Store并返回Local/Remote/Unavailable；跨Host、缺epoch或不完整mirror明确Unavailable，不从最新选择或临时intent推断。Mac队列恢复接该来源；真实Store和原失败native测试待重编验证。
- delegation后续整批为冻结父执行spec→exact started spawn delivery准备→父TaskPromoted提交后才claim→child绑定决定Start或exact Resume→已有CommittedRuntimeDriver执行→seal-only task reconciliation。移动Host仅执行物理请求与报告事实，不另起reducer/Task ledger；目前①②③仍接线中，④移动executor尚未完成。

### 来源 Store 与显式委派命令（2026-10-27，仍待整批编译）

- paired UI surfaces/queued-input/queued-input cancel要求expected_source_store_epoch；Host从同Store Core authority核对，同源Web不接受远端伪造epoch。Apple共用AppleConversationSourceOwner严格解码；iOS来源从Core持久事实读取，排队取消保留原epoch并在响应后核对，surface返回身份同时核对conversation与authority。Android同调用方正在更新。
- 共享child coordinator已在CLI真实worker路径替换原new/resume判断，Task binding存在只能恢复原Run；采用grant、权限恢复与terminal reconciliation均保持两级精确权限边界。六项回归草稿未运行。移动mailbox/独立child Host执行尚待下一层接入。
- 旧/agent直接spawn且未提交TaskPromoted的绕过被列为必须移除：改typed用户委派命令进入Core正常工具/权限链，无首轮模型、无伪ModelCompleted、不转普通聊天prompt；委派动作准入成功不代表child完成。Engine/Application/CLI接线仍进行中。

### Epoch 8 首轮统一检查

- 六包cargo check --tests首轮退出101，日志`/tmp/nola-epoch8-unified-check-r1.log`。CLI显式委派busy检查误放open作用域、旧spawn replay测试缺当前正式HostRequest，两处正在同批修复；不能记整批编译成功。
- 已冻结Domain/Engine/Runtime Driver完整lib回归启动（`/tmp/nola-epoch8-shared-runtime-tests-r1.log`）；与CLI修复文件独立，结果待完成。Web结果/活动14项通过，check零错误零警告，构建完成。
- 当前生产schema epoch8，新验收Host预备5162且强制检查schema_epoch=8；尚未启动。5161 epoch7原库与进程保留，旧证据不得替代epoch8验收。

- 首轮共享回归已结束：Engine 568/568、Runtime Driver 15/15通过；Domain 238通过、1失败。失败为摘要测试用位置下标取错新增后的结果variant，已改为显式Blocked构造，保留reason/evidence摘要绑定断言，待复跑。日志仍保留原失败。
- CLI busy检查已移至具有实际conversation和queued admission的执行入口，open不再引用不存在变量；旧spawn replay测试正在按真实started effect重建。本轮尚未宣称CLI编译通过。

- Domain修复后完整lib回归239/239通过（`/tmp/nola-epoch8-domain-r2.log`）。Application完整lib回归已启动（`/tmp/nola-epoch8-application-r1.log`），尚未得出结果。移动绑定当前仍未连接CommittedRuntimeDriver的原生HostDriver桥，不能把共享worker测试当作移动本地执行验收。

- Application首轮完成：366通过、1失败、1忽略；委派正链在TaskPromoted后authority仍为Prepared（预期Admitted）。正在检查正式source admission export/import是否在fixture遗漏，保留严格Admitted断言。生产CLI已有明确export/import路径，不能以收到TaskPromoted替代可claim授权。CLI检查r2已启动，结果待定。

- 委派准入Application正链r3通过（`/tmp/nola-epoch8-delegation-r3.log`）：真实started effect→prepare冷重放→TaskPromoted→来源导出→事务内可信导入→冷开Admitted及delegation_accepted seal。r2失败暴露fixture缺Application写事务，已补事务，不改变生产准入要求。CLI终态重放r3正在编译运行。

- Application修复后使用当前r3测试可执行文件完整复跑：367通过、0失败、1忽略（`/tmp/nola-epoch8-application-r3-full.log`，16.15秒）。CLI终态冷恢复r3进程仍在编译依赖，未计通过。

- CLI终态冷恢复r4通过1/1（`/tmp/nola-epoch8-cli-replay-r4.log`）：测试隔离NOLA_HOME、DB与project，真实prepared/admitted后failed，冷开拒绝旧请求再执行、保留唯一Task/receipt/command。paired UI epoch真实Core Store测试1/1通过（`/tmp/nola-epoch8-contract-1.log`）。Goal/model resolver的lib过滤各0项，不计验收；Goal已切bin nola目标运行。

- epoch8真实Chrome Goal验收失败（`/tmp/nola-epoch8-goal-real/report.json`）：12次真实点击，开始→真实模型→暂停丢ACK后重试→恢复新Run真实模型→取消202并确认，两个Run均sealed；最后运行记录面板显示无记录，未通过终态历史验收。Host5162原库保留，需修正式trace查询遗漏，不削弱UI断言。

- 运行历史为空的只读证据：连续Goal Run共用原assistant根，messages.run_id为后继；列表条件错误要求每个Run独占messages.run_id，且后继trace.message_id使用后继run:assistant而非原canonical根。正在按真实归属修查询和trace绑定并补连续Goal冷回归，未改验收库数据。子结果真实点击脚本已移除固定旧conversation/2701，要求显式传入本次conversation和预期结果，语法检查通过；尚不计新epoch真实子任务通过。

- epoch8真实模型委派已产生task-1e11b8bccbe1；父conv-20261027005937218-2与子agent-352fa77c14854cf2的Run均sealed。Chrome实时入口脚本仍等待元素，另起只读冷进入脚本验证同一真实任务，未计UI通过。trace修复将使用immutable assistant_turn_message_bindings，不以messages.run_id当前指针证明历史归属。

- epoch8子任务冷UI验证通过（`/tmp/nola-epoch8-child-cold-real/report.json`）：实际CHILD_RESULT=391，刷新保持只读，返回原父会话，新普通会话输入可发送。实时点击验收失败（`/tmp/nola-epoch8-delegation-real/report.json`）：运行中的子Run被observe的run_control_unconfirmed挡住，日志为空；正在分离读取观察和控制权限，不能以冷UI成功替代live验收。
- trace生产修复已冻结待同批验证：idle admission向Engine绑定实际canonical root，历史/exact replay用不可变admission绑定；新增共享root冷读和两轮真实HTTP覆盖。

- 加强Chrome结果断言后同一真实child复验通过（`/tmp/nola-epoch8-child-assistant-proof/report.json`）：只读助手消息明确CHILD_RESULT=391，刷新仍保留，返回父会话正常；用户提示文本不再可满足结果断言。CLI /agent accepted提示改为已委派及实际/tasks入口，不再仅输出Completed误导子任务已完成；待同批编译和PTY验证。

- live child只读修复已冻结待统一验证：observe以Core TaskChild归属+immutable RunBinding证明读取权，显式task_execution、permissions空、can_submit=false，不获得浏览器控制receipt；前端沿现journal读取至seal。增强真实脚本同时核对唯一task/child Run/admitted/terminal receipt，防仅有文本即通过。mobile mailbox/FFI仍编辑中，本批未编译。

- 本批静态审查增加：live child结果先出现不等于seal，真实脚本等待同一绑定Run sealed及唯一terminal receipt后才通过。只读controller终态副作用（完成通知/草稿消费）交由该模块修复者补审，防止读观察伪装用户控制或提交。


## 2026-10-28 收尾：当前证据与真实阻点

本节以当前源码和原故障链为准，旧平台通过数不替代新构建验收。

- Web 本轮类型检查 0 errors / 0 warnings，535/535 回归通过，生产构建通过。`conversationSubmissionController` 的子任务夹具补齐真实协议要求的用户准入声明；原失败保留，未放宽产品准入规则。日志 `/tmp/nola-finish-web-{check-r2,tests-r2,build}.log`。
- 真实 Chrome Goal 已通过：隔离 epoch8 Host 5163，同会话 `conv-20261028114005752-1` 真模型两轮、暂停响应丢失、重载原操作重试、继续、取消与运行历史。报告 `/tmp/nola-finish-goal-real/report.json`，`passed=true`。
- Rust Runtime Driver 完整三组 15+15+16 通过（含有界 mailbox 的开始提交 ACK、foreign ticket、关闭解阻）；Engine/Store 共享 assistant 根历史各 1/1 通过。日志 `/tmp/nola-finish-driver-full.log`、`/tmp/nola-finish-trace-{engine,store}.log`。
- Application 首轮完整 371 passed / 1 ignored。后续真实冷恢复发现问题后代码继续变化，最终全量结果另行记录，不能沿用首轮当最终通过。
- Android 真实模型实际创建本地子任务并进入剪贴板审批，42.7 秒通过；原 user10 冷恢复随后 180 秒失败。根因是 frozen tool manifest 被误当作当前可用性观测。保留原任务 `task-1a99c2df1fd24300b1e3847850215c3b` 和 `/tmp/nola-android-child-cold-store`，不重新提交任务。
- iOS 原生合同 60/60 通过，真实子任务审批等待 65 秒后失败。父任务内部控制工具先 discard 流式 ToolPart 再生成第二 Part，导致严格 source owner 冲突；同原模型输出在隔离原库副本精确重放后已修复通过，Store 所有权检查保持不变。日志 `/tmp/nola-streamed-control-owner-test.log`，原报告 `/tmp/nola-ios-embedded-child-20261028-report.json`；原运行仍需重开后批准并验证精确终态。
- Core 新 `admit_mobile_task_child.current_tools` 必填新观测，按冻结工具身份选取、排序、验证稳定 schema/权限，只容许观测 receipt 刷新；不修改原冻结有效期。父恢复通过 `validate_mobile_run_recovery_authority` 仅恢复 Core 自己注入的 delegation capability，原生工具仍须重新观测且与冻结权限一致。
- Core `load_mobile_task_wake_plan` 给出当前候选、原父调用配置和下次时间，客户端仅负责系统前台/定时唤醒；claim 与到期恢复仍在同一 Application 事务。`inspect_mobile_permission_request` 验证原待审批 effect，允许同一 attempt/token 的已提交续租，不替换执行 owner，不授予权限。
- Mac 来源控制 9 项及源选模失败回滚 1 项通过；paired 显式选模用既有 Core 原子创建/CAS，真实加密 HTTP+SQLite 合同通过。真实 Mac Goal 窗口、丢回执和冷开验证仍在进行。

当前验收未完成：Android/iOS 原待审批链的冷恢复与终态、Mac 真实 Goal 故障窗口、最新构建的 Web live child 只读导航。不得将“已显示权限卡”、传输结束或控制请求持久化记作任务完成。

- 最后共享修复的 Application 完整 lib **373 passed / 0 failed / 1 ignored**，`/tmp/nola-finish-application-final.log`。包括 fresh child tools 只更新 receipt/不能扩大权限、父恢复仅补冻结 Core capability，以及真实 SQLite 准入/同 Run 恢复。此结果仍不能替代正在进行的原生真实冷恢复。

- 最终 CLI 构建通过（`/tmp/nola-finish-cli-final.log`，2m02s；链接器 compact unwind 大小警告保留）。确认原 Host 三个 Run 全 sealed 后，用当前二进制重启同一 5163 Host、同一数据库及令牌；未重置测试状态，live child 验证待运行。

- 当前 Web live child 两次真实失败（`/tmp/nola-finish-child-real{,-r2}/report.json`）已定位：首次子会话不在列表，前端读取 `/api/chat/conversations/:id`，后端遗漏 GET/HEAD 空 suffix，返回405；冷开命中列表会掩盖此问题。已补只读详情路由复用既有 Store/Core 查询，等待最终真实复验；两次原任务与结果均保留。

- Web 详情路由构建通过，当前同库认证 HTTP 已验证 existing GET200、HEAD200、missing GET404。第二次失败验收关闭浏览器丢失 UI owner 后，使用隔离库原 admitted receipt 调正式取消 API，202 后核验所有 Run sealed，再重启同库 Host；这仅是失败测试清理，不计 UI Stop 通过。后续脚本保存私有浏览器 owner 文件以便原身份恢复。
- Android 原冷恢复第三轮进入 attempt2，进一步发现 Activity/ViewModel 销毁的协程 CancellationException 被当成业务 Interrupt，导致原父 durable cancelled 并级联 task cancel_requested。保留原取消证据，不清标志、不冒称恢复成功；正在修产品生命周期与显式停止的边界，后续另建真实待审批任务，以外部 force-stop 验证同任务续接。

- iOS 审阅确认同类问题：AsyncThrowingStream.onTermination 调 cancelStream→submitCancellation。正在改为本地观察/driver detach与用户显式取消分离；原iOS失败库取消标记仍为0，可继续原任务验证。Mac管道onTermination仅关闭读取资源，不沿此路径提交业务取消。

- Mac r4原Goal真实请求先有reasoning_delta及首模型成功，后续provider返回`model_route_changed`，Run已sealed；保留同Goal续测控制，不能把该模型业务失败记为完成回答。原测试仅等text_delta会漏掉真实thinking-only进展，验收信号需覆盖真实reasoning_delta。

- Web live child最终同原r3任务续测通过：`/tmp/nola-finish-child-real-r3-resume/report.json` passed=true，Task `task-1cf01bb3aff3`、Run `34bf54c8-0141-4537-a632-72180c06a49e`。实际打开时running、只读无composer、助手结果391、刷新保留、返回父会话实际Stop、父sealed，唯一child terminal receipt=1。r3初次断言误把已创建但尚在排队的child当成已有Run，修为等待精确binding；同任务同私有browser owner续测，无新输入绕过。
- 取消恢复另发现Core缺口：已bound+cancel_requested的过期Task走普通recover会尝试直接settle cancelled，被“必须由canonical Run seal结算”正确拒绝。补cancel-only准入/泵模式，保留原attempt作身份见证，不普通claim、不续执行lease、不发新模型/工具，仅推进已持久取消与其精确回执，待回归和原库验证。

- 合并cancel-only与Goal能力后的Application完整lib：374 passed / 0 failed / 1 ignored（`/tmp/nola-finish-application-final-r2.log`）；mobile bindings当前check通过（`/tmp/nola-finish-mobile-final-check.log`）。取消准入真实SQLite验证live lease不抢、expired同bound Run、旧attempt不续、不直接settle、仅Cancel待执行、重复准入同身份。各端最终native验证继续。

### Android embedded cancellation recovery — 2026-10-28

- Current arm64 APK and instrumentation build passed (`/tmp/nola-android-cancel-only-final-build.log`, 6m27s); 19 JVM tests passed across original execution policy, permission registry, collector/Stop separation, Goal capabilities, and durable Goal intent recovery.
- Preserved original task `task-1a99c2df1fd24300b1e3847850215c3b`: its parent had been cancelled by the old collector teardown bug. The new Core cancel-only admission recovered that same bound child Run, then produced a canonical `cancelled` seal with no unknown effect references. No cancellation flag was cleared and no replacement task was created.
- Read-only SQLite copies before/after: `/tmp/nola-android-child-lifecycle-live-store` and `/tmp/nola-android-child-cancel-only-store`. Exact Task attempts remained **2 → 2**, model-attempt journal events **9 → 9**, terminal receipts **0 → 1**. The sole receipt source is `bound_run_terminal`, with the original child Run binding; the original second attempt became superseded without a new execution lease.
- New external-force-stop permission test is still in progress. Initial startup failures occurred before submission (missing Compose hierarchy, then sleeping emulator), and are preserved under `/tmp/nola-android-child-external-cold`; they are not counted as a successful cold permission recovery.

- Goal当前wire单项在lib目标实际1/1通过（`/tmp/nola-finish-goal-wire-r2.log`）；此前误选bin目标0项明确不计。Web能力回归11/11、check零错零警告、生产build通过。最终CLI二进制正在构建，随后原库Host更新三能力字段及新Web资源。

- 最终Web全量536/536通过、0失败/0跳过（`/tmp/nola-finish-web-all-final.log`）。最终CLI构建通过，同库Host已更新；Mac只读paired确认原needs_user Goal的can_pause/can_resume/can_cancel均true。Android原取消链额外确认model_attempt_changed仍9条未增加，terminal receipt来源bound_run_terminal唯一一条。

- iOS最终7组当前Core/Swift合同：81 passed / 0 failed / 1既有recorded-trace fixture skip（`/tmp/nola-ios-embedded-child-final-contracts.xcresult`）。原任务真实cold仍待补XCTest克隆缺失原Keychain槽后的复验，不能以合同通过替代。

- Mac最终native能力/回执9项全通过、0skip（`/tmp/nola-mac-goal-current/capabilities-native.xcresult`）。继续原Goal r6真实流程，不创建替代Goal。Android手动界面首轮未启用Agent导致模型未调用spawn，已真实Stop且源sealed/cancelled；不算委派通过，后续经实际设置启用后复验。

- Follow-up manual UI test used the actual Agent Mode switch (enabled and saved) and a synthetic clipboard sentinel, then created parent Run `d79c2526-e039-4d11-8ec0-e3b1e0ffc945` / task `task-ea003dadbea84e76825a5f5351e227f4`. The real clipboard permission card appeared, and an external `am force-stop --user 10` killed the process before approving it. The stopped database confirms `cancel_requested=0`; this validates that process death no longer becomes business cancellation.
- Cold recovery remains **failed/incomplete** at this evidence point: Core reacquired the same Task as attempt 2, but its original permission effect remained `execution_started` past its expiry instead of being recovered/reissued. Exact pending permission: `agent-4a96f4d6a5414d0e83a187f2c5f7f73b:permission:10`; effect `task-ea003dadbea84e76825a5f5351e227f4:host_effect:11`. Shared mobile child pump has no `RecoverExpiredRunDeliveries` call. Evidence: `/tmp/nola-android-new-child-permission-before-kill.png`, `/tmp/nola-android-new-cold-stopped`, `/tmp/nola-android-new-cold-reopened`. The original Run/task are retained for the shared fix; no replacement task or synthetic approval was used.

### 2026-10-28 Mac 原 Goal 续控、共享控制能力与最终证据

- Mac 不再按当前全局执行Host或最新Run投递Goal/子任务控制：Goal读取持久来源，任务指导/取消携带原消息的parent Run；本地取消走typed Application请求，不把取消意图当实际停止。来源/固定parent/冷读取9项合同已通过；新增显式源选模ACK失败保留原选择1项通过。paired `/api/mobile/chat/agent/v2/execution-selection`复用Core `SelectConversationExecution`与原子`CreateConversation`初始配置，真实SQLite+加密HTTP覆盖创建、CAS冲突、错误Host、过期profile、不改全局默认。Goal Start不会暗中保存模型。
- 真实Mac发现Goal为`needs_user`时物理Run仍能输出，而UI只在`running`显示Pause。现由Core `GoalControlObservation`输出`can_pause/can_resume/can_cancel`，Host透传，Mac/Web/iOS/Android消费同一能力；无客户端status兼容回退、无新增持久schema。Core六状态转换1项、CLI wire1项通过；Web相关32项、svelte-check零错误/警告、生产build通过。Mac最终Test构建及9项（Inspector 8、源选模ACK 1）通过：`/tmp/nola-mac-goal-current/capabilities-native.xcresult`。
- 同一原会话`4CEBA0A8-47E4-4189-B9A7-70D3D73646EB`与原Goal`goal-0CA1B9CE-49A5-44BF-996B-5DA6AB9B2D8A`保留到最终，未重建失败库或以新Goal替代。r7生产NSHostingView/NSWindow内实际AppKit编辑器/AX按钮链：冷开原暂停Goal→Resume→新增Run真实reasoning delta→Pause（Host返回202后代理故意丢回应）→按同operation源回执确认暂停→Resume→另一个新增Run真实输出→Cancel→Core cancelled→冷开同mirror/source仍cancelled。`real-r7.xcresult` 1/1通过，0跳过，约56秒。
- 本轮两个新Run `goal-run-e8357bca-e114-4217-bf04-e319fb309b70`、`goal-run-22544113-5886-453c-bf01-85b3af6f96ae`分别产生4、5条真实reasoning delta，均sealed；该Goal全部5个Run已sealed。四个本轮用户操作各只有一次HTTP发送且Core ledger rejection均为null，暂停丢回应后采用权威回读收敛，无重复Pause请求。证据：`/tmp/nola-mac-goal-current/evidence-r7/operation-ledger-audit.json`、`source-runs.json`、`report.json`及`/tmp/nola-mac-goal-current/fault-proxy-r4.log`。不把模型/调度器其他合法ledger记录误算为重复用户控制。
- 失败记录保留：r1环境未传入而skip；r2测试AX setter未写入SwiftUI绑定；r3误等整篇回答完成而超时；r4真实provider `model_route_changed`导致Run失败；r5暴露上述needs_user/Pause能力缺口；r6测试把reasoning delta的`detail_delta`误写为`delta`而超时。最后脚本用`coalesce(delta,detail_delta)`并按原库动态baseline识别新增Run，已在真实库验证旧3个Run。清理API Pause/Cancel与测试失败均不算原生UI通过。
- 图像`paused.png`、`cancelled.png`、`cold-cancelled.png`位于上述evidence-r7目录，是生产Inspector宿主视图捕获；系统shield使CUA精确测试App路径报`cgWindowNotFound`，未绕过系统遮挡。缓存图中原生按钮标签未可靠呈现，故这些图仅证状态与结构，不宣称正常WindowServer整窗视觉验收。功能证据来自实际控件动作、真实网络与源回执。Mac真实子任务指导丢回应、全应用窗口视觉矩阵仍不能由这项Goal测试替代。
- 最终XCTest退出、provider空闲，专用5164故障代理已关闭；root的5163 Host/原DB/token保留，供其他端继续验收。

### 2026-10-28 移动审批过期恢复：共享修复与最终回归

- 共享 child pump 在启动和后继 claim 前调用既有 `RecoverExpiredRunDeliveries → PrepareRunEffectRecovery`，覆盖 Task lease 与 effect lease 不同时到期；不抢仍有效的投递。
- 实际 SQLite 进一步发现 `RequiresRecoveryDecision` 的审批等待不能直接按业务工具未知结果处理。新增窄 Core `PendingPermissionRedisplay` 证据，仅在同一 Store 恢复事务中核对未封存 Run、不可变 outbox 摘要、当前 checkpoint witness 和原 pending approval；不改冻结 replay 字段、不迁移数据、不产生用户许可。普通 mutation 无权使用此证据。
- 原审批冷开回归通过：有效 lease 不抢、到期原请求重示但新 attempt/token、重复准备无重复请求、旧 owner 的允许回答拒绝、foreign proof 拒绝、真实拒绝后不重示。`/tmp/nola-mobile-permission-expired-recovery-test.log`。
- 当前最终 Application binary 全量 **374 passed / 0 failed / 1 ignored**，18.53 秒：`/tmp/nola-finish-application-permission-final.log`。Kernel effect-delivery 12/12通过。移动原任务模拟器验证仍待最终包，不能以此代替真实结果。

- Android 最终包真实恢复同原 permission10/effect11，旧 attempt 中断并创建新投递；实际点击允许后失败，错误 `invalid permission settlement frontier`。保存一致 SQLite `/tmp/nola-android-permission-restored-consistent.db`（允许前）与 `/tmp/nola-android-permission-settlement-failure.db`（失败后），均 quick_check=ok；逐文件拷贝的早期不一致副本不作证据。
- 同原 attempt2 的 Allow 输入依据真实请求/投递/用户操作重建，仅在隔离副本调用正式 ApplicationCommand 重放。诊断证明 Core event revision=27，而 Store checkpoint CAS revision=10；两者不是同一计数器。Store 改为校验前后 Core witness 的版本范围，继续验证原 pending effect、turn 与 cursor，不删所有权校验。修后同次重放提交成功：`/tmp/nola-permission-original-replay-fixed.log`。原设备库没有手改或伪造结果；真实端到端仍待新包复验。

- 版本修复最终回归：正式 Runtime 模型工具流式参数→刷盘→冷开→审批恢复→允许执行（Retry/Resume分别覆盖），明确断言有效 settlement 的 Core revision 大于 Store revision；原Deny与旧owner拒绝仍保留。7/7通过。Store验证器直接拒绝前后Core版本区间外的事件，1/1通过。
- 最终当前 Application 全量 **380 passed / 0 failed / 1 ignored**（18.48秒，`/tmp/nola-finish-application-frontier-final.log`）；Store lib全量 **458 passed / 0 failed / 4 ignored**（69.39秒，`/tmp/nola-finish-store-frontier-final.log`）。这些是共享合同/真实SQLite证据，最终移动原链真实验收另记。
- Android frontier修复后同原任务的permission10真实Allow已提交（host_request15）；随后ExecuteTool17明确返回`The Core tool plan is expired or invalid`，模型自行重试到后继permission request28，子Run仍running，不能算结果成功。父Run已answered但没有child receipt，同样不代表子任务完成。新一致副本`/tmp/nola-android-frontier-after-allow.db` quick_check=ok；App已正常force-stop保留任务且cancel_requested=0。
- 根因为embedded dispatch一直使用child创建时工具plan，遗漏foreground已有的严格fresh receipt重验证。现抽同一`androidToolPlanForHostRequest`给foreground/embedded调用，只有permission/tool执行需要刷新；原身份、工具schema、权限与grant集合必须不变。4项JVM通过（`/tmp/nola-android-child-fresh-plan-tests-r2.log`），APK仅Kotlin增量已安装，真实后继审批待续。初次新测试因固定1970时间与真实clock门禁冲突失败，已仅改fixture使用当前时间，未放宽产品时效。
- 原Android任务最终实际闭环：同`task-ea003dadbea84e76825a5f5351e227f4`内后继读取调用返回`succeeded`，真实内容`CHILD-COLD-CLIPBOARD-842`；child与原parent均sealed/answered，Task sealed、cancel_requested=0、唯一terminal receipt=1。`/tmp/nola-android-final-child2.db`及`/tmp/nola-android-final-child2-audit.json`保存一致源证据；未新建替代task，permission10提交成功与后继重试审批明确区分。
- 最后真实列表点击发现本地child会被当普通会话尝试恢复，显示旧审批与`effect recovery has no active Run`。现Android严格解码Core `execution_ownership`，TaskChild页面不启动前台pump/恢复/权限查询，不展示输入框，发送入口也拒绝；终态普通会话无activeRun时不再恢复未知effect。不是按是否paired或是否配置模型推断只读。
- 新APK冷开同child显示完整哨兵结果、真实失败与成功工具历史，无旧审批/恢复错误；实际返回父页后写测试草稿`PARENT-DRAFT-842`，再次打开child并返回，草稿完整且Send可用，未发送新消息。截图`/tmp/nola-android-final-child-readonly.png`、`/tmp/nola-android-final-parent-draft-return.png`。typed decoder13项+detail reader2项共15/15通过；首次编译smart-cast错误已修，日志`/tmp/nola-android-local-child-readonly-build-r2.log`。仅验证这条真实路径，不宣称全平台每个按钮均已覆盖。

### Mac 指导验收暴露的首次发送前置条件缺口

- 实际 Mac 请求被源 Host 拒绝于 Run 准入前，原因不是模型超时：请求漏带 `expected_configuration_revision` / `expected_parent_message_id`。Mac 普通发送及未绑定 Room 请求现复用已有 `/chat/agent/v2/submission-frontier`，读取原源归属及显示父消息，再将前置条件冻结进请求摘要；已绑定重试不重读。Apple DTO 收到 Shared，iOS删除嵌套重复类型。Mac合同1/1、共享源码Swift6编译及11个边界断言通过。
- 显式选模会先创建源会话；此前 Core frontier 要求任意已存在会话都必须有 admitted Run owner，首次发送无法满足。修为仅本地 User 会话且无任何消息/Run历史可读取首次配置版本。以专用 bounded EXISTS 查询包括隐藏消息和sealed Run，不将配置检查点存在误判为执行历史。原失败库存在 ConfigurationChanged 检查点而messages/runs均0；未删除该检查点。
- 真实SQLite回归包含Create/Select、保存Core配置检查点、冷开读取版本、同CAS准入首次Run、已有历史时无owner拒绝。`/tmp/nola-configured-source-frontier-r4.log` 1/1通过；最新Application全量381/0/1（18.88秒，`/tmp/nola-finish-application-frontier-r4.log`）。前几轮不足的fixture/代理整包缓冲失败均保留，不作为真实guidance通过。

- Mac r6实际首次准入成功：同会话`31C5FD43-8D33-4168-A678-65C505851A46`、父Run`DEB4D1E0-C8B6-4B15-952F-D749A6743392`创建真实子任务`task-15610d9009bf`，子任务自然封存；尚未发送指导，不计指导验收通过。随后同原Run冷观察暴露`missing field schemaVersion`。Mac Application通用`.convertToSnakeCase`会改写嵌套PromptCacheDiagnostic规范camelCase字段，当前正在修复该传输边界；iOS和Android对应路径保留原协议键，不沿用此编码策略。未修改原数据库或放宽Core解析。

- Mac专用ContextUsage命令现使用默认JSONEncoder保留规范字段；cold-r4在同原Run顺序observe已返回、镜像Run已sealed并持久化remote context usage。原生测试随后误用localRunContextUsage查询远程记录而失败，正在改为核对真实remote表，不记整项通过。cold-r3的并发观察冲突另行保留。
- 并发窗口位于Core内容stage：另一receiver已提交同记录后，延迟chunk仍按head+1准备而冲突。补充精确已提交header、原Host/owner、前缀/结果/目标manifest全部验证后的幂等ACK，既有next-content已具备同类已提交检查；不重建stage、不忽略未知冲突。SQLite延迟page、foreign Host及错误bytes回归正在运行，原生新并发路径尚未验收。

- 共享stage修复最终验证：精确大记录/延迟chunk回归1/1，`/tmp/nola-journal-delayed-stage-exact.log`；当前Store全量458 passed / 0 failed / 4 ignored，47.79秒，`/tmp/nola-store-journal-final.log`。首轮错误模块过滤得到0项，明确不计验证。迟到同page成功、错误Host及错误bytes拒绝、无stage残留、head不变；不宣称此项已完成各端真实并发验收。

- Mac最终cold-r7 **2/2通过、0skip**（`/tmp/nola-mac-guidance-current/cold-render-r7.xcresult`及`.log`）。原会话/原Run使用canonical terminal身份（已封存后outbox可合法清理），实际向源Host观察并核对真实`remote_run_context_usage`完整record、digest与`schemaVersion`；diagnostic必须存在，不再允许条件分支跳过。两侧digest均`b6608268af71a3c198f4127f504a0704282011845534992e0f29605a33d0983d`，证据`evidence/source-context-usage-roundtrip.json`。
- 同轮原生产任务卡AX可见、执行过程展开；Mac识别当前wire的`sealed`为终态，显示Finished而非Unknown，不把封存推断为成功，终态文本不再继续按流式处理。交互门禁测试涵盖sealed以及未知/取消状态。截图仅组件缓存且有裁切，不作为整窗视觉通过。cold-r5错误截图字节阈值已替换为图像解码/尺寸检查；cold-r6误要求已消费outbox存在的失败保留。
- **真实指导丢响应仍未执行**：上述测试方法沿用指导测试名称，但本轮运行renderOnly分支，仅证明原任务冷开、源协议/镜像持久化、AX卡片及sealed合同。不得据方法名把指导故障验收记为通过。共享stage并发修复以Store精确交错回归为证，未宣称所有客户端真实并发矩阵通过。
