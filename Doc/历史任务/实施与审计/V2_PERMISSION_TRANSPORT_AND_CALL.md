> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 权限交互传输与原调用生命周期

`PermissionRequestCommand::Intervene` 的独立 `permission_requests` 行属于后台任务 Host 的交互传输。CLI `task_runtime_parts/supervision.rs` 将该行的 typed resolution 读出，`core_agent_runtime.rs` 再提交 `HostPermissionResolution::provide_instruction`。旧交互行 resolved 表示这一条用户回应可投递，不表示 Core 工具已完成或已获授权。

Core 接收 Custom response 后保留原 call、原 arguments、resource 与权限评估，记录有界 `permission_instructions`，发布具有新 request/effect ID 的后继 `permission.ask`。后台 Host 接到后继请求后建立新的待回应 transport 行。旧回应的收据重放不得完成后继交互；只有用户针对后继请求作出明确授权，才恢复原调用，授权产生的 ExecuteTool 携带同一份描述性 instruction metadata。该 metadata 不混入工具 input 或权限 grant。

实际验证位于 `core/nola-application/tests/tool_batch_receipt_e2e.rs`：任务传输 typed custom response → Core HostResponse → 收据与后继 outbox → 冷恢复 → 新 transport pending → 明确 once 批准 → 原 call/arguments → sibling 独立权限。测试也在最终 receipt UPDATE 注入故障，确认旧 checkpoint/outbox/权限记录保持不变。测试运行结果由本轮交接记录列出。


## 2026-09-08 实施边界

- CoreTool 和 delegated permission 均在补充指令后保留原调用，后继交互使用新 request/effect ID。delegated continuation 额外保存原 backend permission request ID，最终明确决策仍发给原 target/backend/session。
- `PermissionInstruction` 是 typed metadata；每条最多 4 KiB、最多 16 条、合计最多 16 KiB（含关联 request ID）。空白及超限回应被拒绝，不消费原等待状态。它既不改变业务 arguments，也不生成 grant。
- 同批其他工具的 conversation 授权不能自动越过已经补充指令的调用；该调用等待自身后继请求的明确决策。
- CLI SOP 继续等待首个 permission reference。补充指令不触发 Replan；后继权限请求可通过 `ValidatePermissionInvocation` 查询校验同一个未结束的 wait。普通工具的 `ValidateInvocation` 仍只允许 Running，错误 reference 不得恢复步骤。
- CLI delegated Host 不再把 Intervention 静默映射为 deny。Core 保持等待，最终仅发送显式授权。Host preflight 错误也不再伪装成用户补充指令。
- CLI Core 权限面板增加独立补充说明入口，保留返回输入框的取消语义；原生/Android/Web 路径增加 typed 指令记录读取与呈现。自然语言说明不自动修改原工具参数。

这些变更修复了原调用状态转换与相关传输接线，尚不能替代五端实际点击、真实 delegated provider 重启恢复、四种 scope、OS policy 分离及完整 §20 验收。客户端测试和最终整合结果见本轮交接。


## 后台任务 transport 的预算前置校验

`permission_requests` 现在独立保存真实 `core_permission_request_id` 与 `permission_instructions_json`；Application DTO 对外提供 typed history。CLI 从 Host 请求逐项透传，不使用 transport ID 冒充 Core ID，也不改写 tool input。历史说明同时进入后台任务权限摘要供用户查看。

Application Enqueue 校验已存在的 history；Intervene 在消费 pending 行前调用 HostProtocol 共用 `PermissionInstruction::validate_append`，Store 在同一写事务中重新读取 pending 行并执行相同校验。每条 4096 字节、16 条与合计 16384 字节的限制均含规范 request ID；超限不改变 decision、resolved_at 或 pending 状态。无 Core ID 的旧式独立请求允许明确 approve/deny/return_to_composer，不能提交 Custom；不存在可安全推导 Core 身份的 fallback。

source schema 版本为 14，遵守现有 hard-cut 拒绝不兼容库策略。bootstrap `open` 默认 `RequireCurrent`；只有显式 `RecreateIncompatible` policy 才会删除不兼容库。本轮测试只使用临时目录或内存数据库。

验收：`permission_request_port_e2e` 覆盖单条、数量、合计预算、非 Core 请求拒绝和 Store 直接调用拒绝后 pending 内容完整不变；Custom transport 的最终 receipt 故障回滚及冷重启 exact replay；`tool_batch_receipt_e2e` 使用真实 Core 原始/后继 ID 和 history 验证 Custom 投递后同 call 保留、批准恢复原参数且不重复发布。CLI 对已经终态的 transport 返回明确错误，避免把未提交回应显示为成功。


## 客户端输入、真实 broker 与迟到回应

Mac/iOS/Android/Web 提供独立补充说明输入，发送 typed `provide_instruction`，不产生 grant、不改写工具 arguments。空白与超限回应在消费等待前拒绝；输入失败保留草稿，回调绑定原 request ID。移动端的 OS DeviceBridge 权限请求没有 Core continuation，因此不展示 Custom 输入。

实际 native broker 的 resolver 现在接收 instruction history，并写入 challenge 与 SSE envelope；只扩充客户端 decoder 不足以实现此契约。后台任务同时持久化真实 Core request ID，后继 Custom 请求不继承旧 task grant 自动放行。

iOS 的 SSE 消费不再等待权限 HTTP 回应；Web 同样允许 p1 回应在途时呈现 p2。迟到成功、失败及旧事件不能清除 p2。Mac 校验实际远程 envelope 的 session、generation、nonce、expiry、trace 和 request/call 关联后展示权限卡；独立 Core snapshot 不能直接推断 live remote challenge 已终止。

broker 接受回应后保留有效期内、最多 4096 条的进程内 exact response ACK；相同回应重试不再次投递，冲突回应拒绝。这只是 transport 投递确认，非持久化执行结果。主机重启、300 秒等待超时仍映射 Deny、Mac 已消费匹配身份的规范 resolved/运行终态；其他客户端的完整终态与跨进程关联仍需验证，不能宣布远程权限恢复完成。


恢复设计的下一步不能仅把 `Deny` 改为 `Err`：现有 `CliAgentHostDriver` 在 permission handler 出错时发送 `permission_ui_failed` 且 retryable=false，仍会结束原 Effect。需要沿 resolver→permission handler→HostResult→Application 的整条链路引入可恢复 transport 状态，保留已提交 ask、区分 challenge 有效期与用户拒绝，再测试超时、断线、主机重启和冷重放。不得用延长超时或返回 Custom 字符串掩盖该缺口。

Mac 规范终态清卡使用请求创建时记录的 generation/conversation/trace/run/turn/request/call/sequence，精确匹配后才撤销卡片。p1 resolved 不清 p2，错运行身份不清卡，终态提高 replay 水位防止旧 ask 再现。无 trace 的 transport 错误保留待确认状态，清卡本身不发送授权或拒绝。最终 Mac 专项 10/10 通过（`/tmp/nola-custom-input-mac-lifecycle.log`）。


跨端 Host 权限 wire 当前统一为 3。iOS/Android 的历史连接常量 2 与 Android 事件分支 1 已删除；旧版本拒绝，DeviceBridge 和 Core projection 的独立版本不混用。iOS 原生 7 项及 Android JVM 26 项最终通过，覆盖真实解码入口而不只依赖客户端常量生成 fixture。完整实机 Host/模型端到端会话仍待验收。


## 权限 transport 中断的非终态提交（后续施工）

本节覆盖上文“300 秒超时映射 Deny”的旧阶段事实。HostProtocol 新增非 terminal `PermissionTransportInterrupted { reason }`，原因是 Timeout/Disconnected/Unavailable；仅 `RequestPermission` 可接收。Core 将原 effect 标记为 `PermissionInterrupted` 并提交 checkpoint，保留原 call、arguments、instruction history 和 permission ask，不发布 grant、ToolCompleted、ExecuteTool 或新的模型请求。`Failed { retryable:true }` 仍为真实失败，未改变其既有语义。

Application 沿现有 nonterminal UoW 提交 checkpoint、回应游标、outbox 与 receipt。冷读保留同一 ask，显式 ResumeHostEffect 可在原 effect 上重绑 delivery 身份；旧身份不得回应新投递。最终 receipt 故障回滚及冷恢复后明确批准只发布一次 ExecuteTool 已接真实 Application 回归。Runtime driver 在一次 pump 中不会忙循环重派同一中断请求。

CLI 的 resolver、permission handler、native stdin 和 desktop permission wait 现在传递 typed interruption，Host driver 发上述非终态回应。EOF、读错、窗口关闭、超时和 transport 不可用不再伪装为用户 Deny；明确 Deny 保留。旧工具执行入口收到中断返回明确 transport error，不能写入拒绝/授权缓存。后台输入中断不调用 resolve。broker 在所有退出路径清理进程内 nonce/waiter；重派保留 Core ask ID、生成新 nonce，迟到旧 nonce 不可批准。

仍未完成的真实恢复边界：持续进程的 supervisor wake-up/backoff、完整 Host 断线重连和 lease 重绑；后台 task + Core ask 的 pending/resolved transport 复用；CLI 新输入前的 abandoned-turn 策略与中断等待需要统一。不能把 driver 冷恢复 fixture 等同于完整产品重连路径。本轮也确保 nonterminal yield 不确认消费 task context，但上层返回/会话恢复语义仍须继续收敛。

## 后台重派恢复同一 Core ask

`PermissionRequestCommand::EnsureCoreRequest` 是写命令，返回规范 transport `PermissionRequest`。Store 同事务按 `(task_id, core_permission_request_id)` 复用 pending 或 resolved 行；比较 child session、capability、scope、语义 input 与完整 typed history，冲突拒绝。新候选 transport ID 和创建时间不覆盖已有身份。schema 15 新增这对身份的部分唯一索引，非 Core 独立请求不受该索引限制；默认不重建用户库。

CLI 后台等待实际调用 Ensure，并使用返回的旧 request ID 继续读取。resolved 但未送达 Core 的 Custom/deny/allow 会再次投递给同一 Core ask；已有 task grant 不覆盖这份决定。只有不属于后台任务时返回无处理结果，Ensure 冲突或 owner 丢失明确返回错误，不回退其它权限入口或合成 Deny。

`permission_request_port_e2e` 新增 final receipt 故障零新增行、冷重启收据重放、pending 与 resolved 并发重派只保留一行，以及 child/capability/scope/input/history 冲突拒绝。它验证 transport 仓库恢复契约；进程调度器重新启动 task 与 Host nonce 更新仍须由实际生命周期入口共同保证。


## 实际 Host 恢复入口与运行结果（2026-09-09）

执行接口返回 `CoreRunOutcome { turn_status }`，不能用 `Ok(())` 推断完成。无输入 resume 在存在 canonical active Run 时可以重复 pump，不再受 `resume_pending_once` 限制，每次绑定当前 resolver/取消上下文。权限仍 pending 时，新输入被明确拒绝且不提交 abandoned-turn Interrupt；显式取消仍走 Application。CLI Goal 不把 nonterminal 部分输出送去完成/audit，后台不 finish task 或覆盖并发决定的等待标记。Web `[DONE]` 仅结束 HTTP，规范状态仍由 Core projection 决定。

恢复在 pump 前经 Application 提交 `ResumeHostEffect`，重绑 delivery 身份。相同 permission mutation 是投影 no-op，因此 Core 在新的事件 cursor 下重新发布已提交的 Waiting Part（保留原 Part revision、call、input、ask），从而生成新的 canonical permission trace；不是伪造工具动作或授权。CLI 原先将 trace call ID 改成 ask ID，现已删除。native presentation 暂存按规范 ask ID 索引，最多 128 项、每 trace 32KiB，同一批请求可反序消费；缺失/错误身份、旧序列、同 ask 改 call/tool 被拒绝，scope 与运行身份隔离。

真实入口回归使用实际 HTTP provider stub、完整 CoreAgentHost 和临时 SQLite 的进程内 Application：首次模型工具调用→中断→新输入拒绝→同进程两次 resume（均必须消费 fresh canonical trace）→仅清 fixture cache 冷 open→明确批准→原工具完成一次。它不冒充真实 LLM 或 Service IPC 测试。普通 CLI resume、Web/Mac 的用户可达无输入恢复端点和持续 supervisor wake-up/backoff 仍需继续接通；只读 trace replay 不能重新建立已经失效的 nonce。

## 显式无输入 Web/mobile 恢复入口

新增 `POST /api/chat/agent/v2/resume` 和 `POST /api/mobile/chat/agent/v2/resume`。请求仅接受 `conversation_id`、`expected_run_id`（规范 Run，不能使用 trace run ID）、`expected_host_id`、`client_id`；拒绝 prompt/messages/context/model/submission/replacement 字段。Web 沿现认证/CSRF，mobile 沿现 bearer+加密 JSON；同 session 执行注册不可替换已有执行。后端调用锁内核对 expected Run 的 Core resume 入口，只发送已有规范 projection/permission SSE 和 transport `[DONE]`，没有伪造终态帧。

重启维护现在按规范 checkpoint 与未 ACK RequestPermission outbox 的精确身份、参数和指令历史关联保护可恢复等待，包含 Host 已 Accepted 后突然退出的情况。仅保护 `trace_journal_runs.turn_id` 对应同一规范 Run 的 trace；其它孤儿和损坏/不匹配事实继续原清理。Store helper 是存储关联事实投影，不执行 Core 归约或授权；完整 checkpoint 仍由 Application/Core 在恢复时验证。

Host-owned 恢复的成功 handler 集成 fixture 已加入 `permission_resume.rs`（真实 HTTP provider、初始 broker timeout、恢复 SSE 新 nonce/trace、更后显式批准原调用）。其运行结果以本轮交接为准。冻结配置不匹配必须明确拒绝；缺少 Device manifest/resolver 的恢复不能被解释为已支持 Device 执行，Device 重新绑定仍是独立待完成契约。

## 冻结执行绑定与恢复后的规范发布

普通 CLI `--resume` 现先无输入恢复已选会话；原 Run 仍等待时不进入新输入框。
远程恢复必须携带预期规范 Run ID，获取 Host 前与锁内都核对身份。热 Host 继续使用
原 Run 的内存执行配置；冷 Host 在探测 Provider 或发布权限 challenge 前验证持久化
`HostInvocationBinding`。绑定随实际 BeginTurn 保存，也在每轮配置更新时保留。

绑定覆盖实际 Provider、endpoint、fallback、模型、生成参数与执行策略；凭据、URL query、
prompt cache key/prefix 只保存 SHA-256 比较值，不保存明文。fallback 的可选 model/key
沿真实执行语义继承主配置；新轮缓存签名覆盖绑定，避免参数变化后继续使用旧 driver。
Device 广告的 schema、capability、egress receipt/grant 纳入摘要，广告缺失/改变或 resolver
缺失时拒绝恢复，不把原设备调用降为其它执行器。缺绑定的旧活跃 Run 不能自动恢复。

恢复完成后复用 `core_conversation_projection_updated` 发布原 Run 的规范会话投影，
供远程客户端通过 Application 提交原占位消息的完成状态。非终态仅发布已有等待投影，
`[DONE]` 不意味着 Run 完成。真实 handler 测试曾发现此规范完成帧缺失，补齐后通过；
没有删除该断言。CLI 原调用、配置漂移/Device 校验与成功 handler 的最终结果见交接记录。

这仍不是任意冷启动配置恢复：当前服务配置无法解析出与旧 Run 一致的执行绑定时明确拒绝，
还需接通版本化配置/凭据解析与 Device 重新绑定。持续 Supervisor、完整跨进程 lease
生命周期、五端真实重连/模型/UI 与全部架构验收仍未完成。

## 后继 Core Turn 与恢复交付身份（2026-09-09）

一个规范 Store Run 可以承载多个 Core Turn：排队的 NextTurn 输入在前轮完成后启动，
不创建新的 Store Run。首次提交 ID 标识原 Run；当前 Core Turn ID 和 diagnostic trace run ID
各自保留含义，客户端不能推断它们始终相等。

无输入恢复 SSE 的规范事件增加顶层 `canonical_run_id`。服务端先核对 Application 活跃
Run、锁内 SessionStore 身份及冻结 Host 配置，随后为实际恢复交付绑定该 Run。权限 broker
直接发出的新 challenge 也携带同一字段；原 trace 和 call/Turn 身份不改写。未通过身份校验
的运输错误不伪装成已绑定的规范事件。

Mac 恢复按该字段与会话校验。中途 Core Turn 完成不会结束规范 Run；只有原 submission、
会话、规范 Run 都匹配的终态会话投影，且本地 Application 提交成功，才能清除恢复状态。
缺身份/旧 Run 事件不更新恢复，运输错误和提交失败保留等待。

另补齐两个持久化边界：普通 Application execute 收到 typed PermissionTransportInterrupted
时立即提交 checkpoint，即使该响应非终态且未达到流式字节阈值；启动维护通过同一 Store
Run 的 checkpoint 和 pending Effect 验证实际 Core Turn，再决定保护哪个 trace。
不能直接用 trace Turn ID 查 Store Run，否则后继轮的合法等待会被当作孤儿。

真实 HTTP provider fixture 已验证首次及后继 Turn 的冷 Host 恢复、错误 Run 零副作用、
fresh nonce、原工具执行一次和原 submission 的最终投影；不是对真实 LLM 的验收。
本阶段完整结果和仍未完成项以最新交接为准。

后续通用恢复已迁入 Application/Kernel：Store 的权限专例和历史终态推断删除，权限原参数/
指令核对在 Application 的当前 checkpoint/Effect 验证中完成；Run 与 trace 在同一恢复事务
应用。实现、拒绝语义和真实进程验证见
[当前 Run 恢复迁移](V2_RUN_RECOVERY_KERNEL_CUTOVER_2026-09-09.md)。
