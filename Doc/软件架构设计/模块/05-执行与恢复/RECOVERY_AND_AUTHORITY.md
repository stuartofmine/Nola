> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 权限、执行权、取消与恢复契约

状态：已核对下列实现入口；跨端调用与完整故障注入未验收。日期：2026-10-11。
细化 MOD-05/07/10/14、LC-02/05；与 COMPLETION 共用同一个 RunResult，不另造终态。

## 三种权力不得混用

### A09：typed 资源到授权消费的生产接线契约

状态：设计已对照当前入口核查，尚未完成生产接线。Domain 的
permission_resource_plan/permission_scope 类型存在，不代表下面流程已经实现。
当前生产仍使用 HostPermissionResourceDeclaration 生成字符串主体及 Core 规则。
本轮已移除后台任务对新 Core permission wait 的 Host 相似规则自动批准，以及
Store 按 Host scope_rule 相同代为批准兄弟等待的分支；验收见 VERIFICATION。
这不代表 typed 资源、frozen offers 和执行前身份复核已经接线。

#### 唯一所有者与边界

| 边界 | 输入 | 输出与写入权 |
|---|---|---|
| Core 规范化 | 已准入 capability 声明、规范化参数、原调用身份 | invocation digest 与 unresolved demands；不接客户端自报 digest |
| Core → 目标 Host | resolution_id、execution_target、invocation_digest、capability、operation、完整 demands | 已提交的解析 effect；此阶段不授予执行权 |
| Host 解析 | 原 effect 的 delivery 身份、资源定位符 | PermissionResolutionFacts、逐需求 evidence_id；不得添加 grant 或替换目标 |
| Core 接受事实 | 原 effect/attempt 来源、完整 facts、冻结参数 | 唯一 ConcretePermissionSubject、策略判断、可选范围决定 |
| Application 绑定 | Core 已提交的决定、实际 Run/Turn/effect/source proof | 带 canonical occurrence 的 FrozenPermissionOffers；只绑定，不扩大语义范围 |
| Store 提交选择 | 原 command identity、occurrence、offer_id、来源与开放状态 | 原子 selection receipt、必要 grant、授权继续的 effect；不接新 scope/pattern |
| Host 执行 | 原已准入调用、所选范围、资源证据引用、delivery fencing | 执行前资源身份复核与实际 receipt；不再维护独立 task matcher 决定允许范围 |
| 客户端 | canonical offers/等待投影 | 展示与选择；不构造 subject、目录范围或 grant |

FrozenPermissionOffers 包含 CorePermissionOccurrence.run_id，所以它是 **Application
绑定后的 canonical DTO**，不是 Core 的原始 effect。Core 只决定主体与范围候选；
Application 读取原已提交决定及真实 Run 绑定后计算最终 offer_id 并保存。冻结结构
的自洽 digest 不是来源签名，也不能证明候选范围经过 Core 批准。
绑定必须与 permission_request_sources 的不可变 proof 同事务关联，不创建另一份
可由客户端覆盖的授权真相。未来 Core 原始 offers DTO 不能复用此带 Run 的结构。

#### 接受顺序与错误分类

1. Core 先验证 capability/operation 与规范化输入；选择器缺字段、空需求、重复
   requirement/item index 属于 invalid_declaration，不请求授权也不执行工具。
2. Application 验证原 resolution effect 的 Host、attempt、generation/lease 和
   delivery identity；错误为 source_mismatch/stale_delivery，拒绝且不消费响应。
3. Core 校验每个 demand 恰有一个同类型、同 access、同定位符的 binding；漏项、
   多项、错目标或 invocation digest 为 resolution_mismatch/incomplete_resolution。
   Host 接触不到目标返回明确 resolution_unavailable，不伪装成用户拒绝。
4. 只有完整主体才能求授权。需要决定才提交 permission wait；已有 grant 也必须
   覆盖该目标、操作及所有资源。单个资源获准不代表多资源调用可部分执行。
5. Store 在一个写事务中核对 selection command、原 occurrence、offers digest、
   wait 未关闭/未撤销、取消及终态条件；同 ID 同内容返回原 receipt，异内容冲突。
6. Host 在副作用前复核 evidence 指向的资源身份。失效属于 stale_resource，不能
   重新解析到不同对象后悄悄沿用旧批准；已有副作用但终局不明走 unknown_effect，
   不以 stale_resource/未执行为由自动重试。

解析失败、等待授权、拒绝、授权过期、未知副作用、任务未完成分别表达，不统一
变成 validation_required。拒绝旧响应不能清除原 pending 或生成新 effect。
路径显示文本不是资源身份：相对路径依赖被冻结的 cwd，远程路径在目标 Host 解析；
符号链接、文件系统、大小写语义及不存在目标的父目录身份都必须由目标解析器证明。
通用字符串归一化不能代替这些事实。

#### 同一纵向改动的交付门槛

- 扩展现行 Host effect/response 和 Core pending 状态，保存解析请求与来源；不能
  在 CLI 私有代码先算一个主体，再把它当作 Core 决定提交。
- 将目标 Host 的解析、Store offers/selection 事务、grant 消费和执行前身份复核
  接成一条链，再删除旧字符串/Host task matcher；不保留双判定回退。
- 进程执行能力需冻结 program/script、参数、环境和 cwd 的完整 invocation 身份；
  当前 ResourceLocatorKind 仅有 file/network/platform，不能假装已经覆盖 process。
- 测试至少包含多资源移动/复制、目录后代、符号链接变化、相对路径跨 Host、进程
  参数改变、自动任务来源、取消与批准竞争、重启重放、重复选择及 UI 会话切换。
  每项须核对实际 effect 数量、持久 receipt、最终资源及客户端等待关闭。

先在同一隔离 workflow 测量解析/批准提交/执行前复核的新增耗时和请求数，再决定
可接受预算。人工确认时间单列；不能跳过身份复核或复用失效证据来达标。
当前缺少上述生产证据，A09 保持未完成。

#### 当前后台任务传输的权限边界

Core 发出 RequestPermission 后，后台任务适配器必须落库原 occurrence 并等待其
明确决定；不能因为 Host task grant 的命令前缀或目录匹配而返回 AllowTask。
AllowTask 只转为 AllowConversation 选择，不携带 Host 自拟的扩大范围。
Core 在自己的 checkpoint 中持久化并复用许可；没有被 Core 覆盖的请求仍需决定。
Store 原子保存所选请求与其派生记录，但不为 scope_rule 相同的其他 occurrence
生成用户批准。即便 capability、输入和 Host 显示规则恰好一样，也不是 Store
代行 Core 策略判断的凭据。历史选择、撤销、重复提交和取消竞争语义保持独立。
该收敛会使之前被 Host 擅自扩大的相似操作重新询问；扩大许可必须由 typed offers
明确授予，不能恢复 Host 自动批准来消除提示。

### 会话授权不是响应方声明的任意范围

当前 Core 将 AllowConversation 绑定到原挂起请求的具体 capability/resource。
响应中附带的 grant 是 Host 执行材料，不能凭其中的 resource_pattern 替换 Core
主体。批处理中的其它挂起调用逐个按原来源和现行规则重新求值；不同文件不因
同批、同工具或 Host 自报 `*` 自动获批。尚未匹配的请求保留原 request identity，
不重复弹出新请求，不写批准回执、不派发；各自明确批准后才产生各自回执和 effect。
更宽范围仍须由将来的 Core frozen offers/用户 selection 完整链路授予，不通过
恢复旧字符串扩权实现。回归 `conversation_grant_cannot_expand_to_a_different_pending_resource`
以两个不同文件验证该边界；不能将它解释为完整 typed 权限接线已经完成。

### A08：未知副作用的唯一恢复决策

`ResolveUnknownEffectRecovery` 是用户控制命令，不加入模型工具目录。每次决定携带固定
resolution_id、conversation/Run/config 身份，以及 effect/request/physical attempt、原
worker/generation/lease token 和预期 revision。三种选择不互相冒充：

上述身份是原样匹配的有界非空值，禁止首尾空白和控制字符；入口不得 trim 后替用户
换一套身份。缺少 fence、未知字段或无效 revision 在任何恢复修改前拒绝。自由文本理由
不参与身份推断，也不能取代已提交 Host 证据。
身份最多 512 UTF-8 字节、targets 为 1–256 项、用户理由非空且最多 4096 UTF-8 字节；
这些输入边界由 Protocol 统一验证。Application 不再另设不同的理由长度上限，负责核对
持久执行事实、目标唯一性、风险重试单目标及受阻决定的完整 pending 集合。源测试分别
覆盖 inspection 必填 owner/fence、拒绝未知字段和多字节理由边界，尚未执行。

- `verify_applied_receipt` 只观察已经通过规范 Host 接收链提交的成功 terminal receipt；
  校验原 envelope、event identity、attempt 和 settled ledger，不接用户文字/模型声明造成功。
- `authorize_risk_retry` 是用户明确承担未知结果风险的重试授权，不是“证明未执行”。原命令
  经过 Core 生成恢复事实；Store 核验该事实及原 submission 后才 CAS unknown→interrupted/retry。
  通用 attempt mutation 路径也核验来源，不能绕过专用命令。
- `keep_unknown_blocked` 保留 OutcomeUnknown。Application 核验全部 pending effects 均在精确
  targets 内；Core 还拒绝独立工作、开放问题、权限等待与排队输入，然后正常决定 Blocked 并
  经已有 seal 事务关闭 Run。不会取消其它工作，不合条件则拒绝而非提前封存。

同一事务保存 resolution receipt；相同 ID+完整请求 digest 返回原 receipt，不重放动作；
同 ID 不同请求返回 conflict。通用 active/idle submission 拒绝直接构造恢复 submission。
Core 原事件、原 Host history、checkpoint、attempt CAS、结果 seal 与 receipt 同 UoW。
ApplicationRuntime 窄接线传入精确热 Core 与已缓冲原事件，不能将旧缓存投影当作恢复事实。
旧 WorkStep `ResolveUncertain` 与 CLI `/work resolve ... applied|not-applied|abandon` 已删除，
不能再用说明文字生成执行成功。旧调用的反例保持 checkpointed 原记录。当前仍待统一验收，
CLI 已接 `/recover` 检查、`/recover retry <attempt-id> <reason>` 与
`/recover block <reason>`；无保存模式在打开持久端口之前明确拒绝。检查读使用同一 pinned
snapshot，要求 exact local assistant invocation、有效 live Run，以及 attempt SQL 索引列与
record_json 完整一致；unknown 必须已有 execution_started/finished 原事实。
权限 fencing 不显示在终端列表。风险授权只提交恢复决定，不立即执行；`/resume` 实际打开
会话选择器，选择原会话后才走既有恢复检查，不能将新消息视作恢复指令。

CLI ApplicationPort 单次执行保留固定 request_id，丢失回执时查询同一 command receipt，
不另造写请求；Domain resolution_id+请求 digest 又约束重复提交。用户重新手动输入命令属于
新的决定，必须重新检查最新 revision，不宣称自动幂等重试。其它端的恢复控件与统一验收
仍待完成，不能仅凭此 API/CLI 源码宣称完整 A08 已验收。

### 热状态拒绝与事务回滚

`RuntimeSupervisor.take_current` 先检查 conversation/Run/Store revision，错配不删除原 owner。
取出 Core 时保留恢复胶囊：原 checkpoint revision、配置 digest、Core revision/event cursor、
Store event sequence 与已接受的 normalized Host replay facts。胶囊不复制 Core、完整上下文或
不断增长的 PartUpdated 快照。热尾部仍受 64 KiB wire 预算约束（每响应最少计 1 KiB，故不足
64 个响应）；胶囊另含有界协议身份和双份编码事实，不能把 wire 预算误称为全部堆内存大小。

无事务热命令拒绝后，或 SQL/savepoint 已确认回滚后，Application 从原规范 checkpoint
恢复，再使用原绑定、序号、决策时间纯重放此前已接受的尾部，并核对 Core revision/cursor。
重建的 deferred journal facts 留待后续正常提交；不派发效果、不重新发布 UI 事件、不重新授权。
成功提交并采用新状态后才丢弃胶囊。重建失败、rollback 失败或 COMMIT 回执未知都 poison
当前 Runtime，要求重新打开，绝不把无法确定的提交状态猜成成功或回滚。

性能取舍：正常 delta 路径只复制有界 replay facts，不按每 token 克隆整段历史；重建完整 Core
成本只发生在拒绝/回滚路径。新增生产测试源码在同一 Runtime 注入无 UoW 参数拒绝、Host inbox
SQL 故障和 command receipt 故障，最后验证原流式尾部仍在、失败文本未泄漏；尚未统一运行。

### 恢复不得另造终态

消息渠道的 inbox 回复投递与 Run 业务结果相互独立。已经取得并接收精确封存记录后，
Host 进程后续非零退出不撤销该事实，也不允许重新执行原消息。包含未完成、受阻、失败
或取消说明的公开回复仍可投递；若没有可投递正文，应报告不可重试的投递失败，而非
重新启动任务。只有回复投递成功才确认 inbox，不借此宣称任务要求已经验收。

Run 的 `sealed` 与消息的 completed/failed/cancelled 是不同维度。只有原 Core
结果决定及其来源事实经过 Store 核验，并与 checkpoint、结果索引和封存记录原子提交，
才能关闭 Run。启动恢复不允许根据 checkpoint 的 Turn 三态直接更新 Run 或消息。
发现终态 checkpoint、没有待交付 effect、却仍是活动 Run 时，返回
`MissingTerminalCommit`，保留事实供原提交恢复检查；不得猜测业务结果或补造成功 seal。
已提交的 sealed Run 不进入活动 Run 修复集合，历史 trace 整理不能改写其结果。

取消命令必须携带 `cancellation_id`，由命令接收边界分配一次，与原 submission 一起
持久化；Core 取消事实引用同一身份，Store 核验原命令，重放不得重新分配身份。
恢复中收到新输入不是用户取消授权，不能仅凭旧 Turn 尚未结束就自动终止它。
CLI 已移除两处“新输入自动取消恢复任务”路径；非终态旧 Run 保持原执行权，拒绝启动
新的 Turn，并要求恢复或显式取消。此实现尚待最终真实恢复链路验收。

开放用户问题也是合法的恢复工作，不要求伪造待派发 Host effect。Store 在恢复写事务的
同一快照读取该 Run 的原始问题来源；Application 将这些记录与重放 checkpoint 的开放
问题逐项比对，校验 conversation/Run/Turn、schema、完整请求及集合一致性。只有校验
通过后，Kernel 才用开放问题数保留非终态 Run。缺失、重复、异主问题或终态仍留开放问题
均拒绝恢复，不以单个 waiting 标签跳过完整校验。恢复不会调用模型或自动回答问题。
本批补充无 pending effect 的真实问题冷重开回归源码；尚未执行。

HTTP 发送线程的响应通道断开是本地传输故障，不是取消证据。发送层返回
`WorkerDisconnected`，适配层保留类型并生成 `transport_worker_disconnected`；
不从其展示文案再判定取消、提供商配额耗尽或自动切换模型。该故障不能证明请求
尚未送达；当前不会自动重试。若同时存在明确取消上下文，取消控制优先。
接收循环和类型映射反例源码已补，真实故障链路尚待统一验收。

Supervisor 的热缓存只按精确 Run、conversation 与 Store checkpoint revision 取用；
revision 不匹配即弃用，不构成第二套恢复事实。提交后的新 Run 缓存替换同会话旧项，
终态 retain 移除旧活动项。旧 `discard_recovered_runs` 已无生产调用并移除，避免恢复
代码再维护一套主动清理清单；事务回滚不发布 staged retain 的覆盖仍保留。

内部后继准入在 Store 写事务内再次检查：精确前驱属于该会话、已经 sealed 且存在
本地 seal 或已提交的远端 seal，同时会话无活动 Run。Application 的预检查不能替代
此事务栅栏。活动前驱反例检查原 Run/checkpoint 不变且没有创建后继；源码已补，未运行。

普通模型正文结束后，Kernel 不因 validation/recovery/skill 待办布尔值额外请求模型
或制造执行失败；它将正文交给 Core 的统一结果仲裁。未完成要求与证据不足必须留在
业务结果中。协议错误、空输出、截断续写和模型明确选择的独立审查仍有各自明确流程，
不能把它们当作补齐业务要求的隐式循环。

同一规则适用于工具执行后的空正文及已接受的独立审查候选：已有验证、恢复或 Skill
待办时，不能把待办转成重试三次后的模型错误。它们交给 Core 仲裁保留未完成／受阻
证据；审查只有明确要求继续执行时才重新进入执行流程。纯协议空响应、无业务待办
的空最终总结仍按有界协议恢复处理。此分支整改已补测试源码，尚未运行。

| 权力 | 所有者与载体 | 不代表什么 |
|---|---|---|
| 用户授权 | Core policy、Application 保存 request/grant/decision | 不证明 OS 允许，不证明操作已执行 |
| 派发权 | Store 已提交 effect + attempt/worker generation/lease | 不授予新的资源权限；租约过期不证明副作用未发生 |
| 结果证明 | Host receipt、版本与 check，经 Core 核对后提交 | 不扩大授权，也不由“已发送/已写入管道”推断业务成功 |

自动批准只影响产品层 Ask 的处理；硬性范围、委托权限上限、OS 约束仍有效。
资源无法确定时不能凭自动批准猜测授权范围；返回明确 capability/parameter limitation。
拒绝、等待授权、OS 拒绝、效果未知、检查未通过必须使用不同原因，不能统称“命令失败”。

### 当前配置准入

权限配置只接受 `cautious`、`allow_all` 两个规范值，不迁移旧拼写；默认谨慎。
首次缺省可初始化默认模式；显式空值、未知值、错误类型或损坏 JSON 必须报配置错误，
不能悄悄变成默认权限。CLI 环境值在配置准入时解析为类型，后续策略消费该类型，
不再次读环境并猜测模式；删除从 auto_approve/network_policy 反推第三种模式的路径。
Web、iOS、Mac 的持久设置读取和实际执行入口同样拒绝无效显式权限值。
读取失败不改写原设置、不撤销已有 grant，也不产生新授权。展示层占位不能替代执行准入。

本批已补配置缺省/损坏、读取不改库的回归；各端统一测试结果单独记录于 VERIFICATION，
不能把解析器通过等同于全权限流程验收。

## 当前类型与入口映射

### 会话删除与不可变记录的生命周期

会话拥有 Run、输入应用索引、执行证据索引及会话内后继准入。不可变表示存续期间不得
单独改写／删除事实，不表示用户永远不能删除整份会话。生产删除入口先拒绝任何非终态
Run，再在同一事务发布同步 tombstone 并删除会话；其派生索引通过外键一起删除。
输入应用／后继准入的删除保护在所属会话、Run 与来源仍存在时生效；来源生命周期结束
引发的派生索引级联必须允许，包括归档恢复在已检查无活动 Run 后替换全部历史。
跨 Run 索引不能依赖前驱／后继的 SQL 删除顺序。不能关闭外键绕过保护。
删除失败必须回滚 tombstone 和所有关系，不留下半份会话。同步 tombstone 和完整投影
替换与本地入口使用同一个事务内活动 Run 检查。

任务准入权威、任务 Run binding 和终态回执不属于该级联生命周期：它们保留已执行身份，
不因删除子会话而允许同任务再次启动。客户端不能以“找不到 Run”解释为“尚未执行”。
本批新增直接篡改拒绝、活动会话删除拒绝、完成后生产删除、外键完整性及任务不重复准入
覆盖；尚待统一运行，不构成已验证声明。

### 后继 Run 与提交回执必须同事务准入

NextTurn 的源提交 ID 不随新 Run 改写。存在远端 source receipt 时，新 Run 准入必须
验证同一 conversation/user/assistant/request 身份及可准入阶段；成功提交同时将 receipt
从 Prepared 推进为 Admitted，并绑定真实消息准入游标。不存在 source receipt 的本地
提交不需要创建虚构远端回执。后继自身的幂等身份检查先于新准入，重复 payload 返回原
结果；不同 payload、已拒绝／取消的源提交不能借后继路径重新执行。

Run、输入应用、后继唯一绑定、消息准入与 source receipt 任一写入失败必须全部回滚。
接收端仍要求已认证 root，不能用“来源创建了 Run”替代 Admitted 证明，也不能自行将
Prepared 提升为 Admitted。本批修复已覆盖 source gate/commit 同事务与故障反例，待复验。

### 多步写入与后台发布的事务边界

Repository 多步修改允许被 Application 工作单元调用，也允许独立调用；独立调用必须
先取得 `BEGIN IMMEDIATE`，再读取版本、成员集合及旧值。已有工作单元内使用嵌套
savepoint，不提交外层事务。模型端点编辑、目录替换、参数配置及批量重置共用这一写入
准入，不允许裸 savepoint 在自动提交状态下产生延迟读转写竞争。

业务事务回滚保证其事实、同步 outbox 和发布目标代数不变；不保证整个数据库在观察
期间静止。归档 worker 独立提交 published generation、重试及错误元数据，其成功发布
代数必须单调且不超过 target。回滚验收分别检查业务事实不变和后台状态合法，不能将
后台合法推进当作部分业务提交，也不能因后台并发就忽略整个发布表。

### 能力发现不是权限授予

Host 激活响应必须携带本次所选 ID 的完整 descriptor 集合，包括 Turn 开始时已存在的
能力；禁止 Core 从旧 catalog 补缺。Projection 校验所选 ID、descriptor ID 的精确集合、
重复项、可用状态和 Tool 路由，整批通过后才替换描述。中途解析或校验失败不能留下部分
更新；上层提交还必须保持原 checkpoint 和响应游标，修正后的同一响应可以重新提交。
这只改变下一次模型请求可发现的工具，不产生 grant，不绕过具体调用的参数和权限检查。

实现入口：`capability_projection::install_activated_descriptors`、
`runtime/tool_driver::install_capability_activation_descriptors`。本批补充缺失已有 descriptor、
后段非法/重复/额外 descriptor、失败后同序号修正重试测试；统一验证尚待执行，不能据此
声称完整真实 Host 或客户端链路已验收。

| 职责 | 实际代码 | 当前事实 / 缺口 |
|---|---|---|
| 策略输入 | `core/nola-domain/src/permission_policy.rs`: PermissionSubject/Rule/Evaluation | capability/resource，规则来源含全局/Host/会话/Turn；主体身份由外围上下文提供，需核对全路径 |
| 产品授权命令 | `core/nola-application/src/permission.rs`: PermissionRequestCommand | Authorize 和 Intervene 分开；授权有 once/conversation/deny，补充指令不是批准 |
| 持久请求与 grant | `core/nola-store-port/src/permission.rs`: StoredPermissionRequest/Grant | grant 有 scope、资源模式、revoked_at；request 有 Core 绑定，但 API 还允许无绑定的旧形式 |
| 请求决议落库 | `core/nola-store/src/permission_transport.rs`: resolve | pending 条件更新；不存在和已 resolved 都返回 false，不能单凭 bool 区分幂等 ACK 与冲突 |
| 委托权限传输 | `core/nola-application/src/delegated_permission.rs`: response_mutations/prepare_effects | occurrence、invocation、generation、request digest；决议与传输证据分开 |
| 派发 attempt | `core/nola-domain/src/effect_delivery_attempt.rs`: EffectDeliveryAttempt | 身份、前驱、revision、lease、start/finish、replay safety、manual resolution |
| 过期/死亡恢复 | `core/nola-application/src/run_recovery.rs`: recover_observed/recover_expired | 校验精确 Run、冻结 Host 绑定、当前租约；过期不是整轮死亡 |
| Run 自动恢复策略 | `core/nola-kernel/src/run_recovery.rs`: plan_run_recovery | pending effects 保留；自动恢复不替用户作风险决定。明确用户出口走本页 A08 专用协议 |
| 取消准入 | `core/nola-application/src/run_cancellation.rs`: prepare/complete | 精确 conversation/run/assistant/cancellation ID，区分已取消中/已终态；不以 PID 为完成证据 |
| 取消交付 | `core/nola-application/src/cancellation_delivery.rs`: supports_dead_owner_confirmation/claim_cancellation_host_requests | 死 owner 仅可确认部分目标；不会仅凭死亡确认已开始的外部工具副作用停止 |
| 未知效果人工核对 | `core/nola-application/src/unknown_effect_recovery.rs` | 精确 owner/attempt/revision、Core 来源事实与 receipt；风险重试、保留未知受阻、原始成功回执核验分开，不从说明文字推断未执行 |
| 查询恢复事实 | `core/nola-domain/src/run_recovery_inspection.rs`: RunRecoveryObservation | Missing/IdentityMismatch/Inconsistent/Active/Terminal；查询不授权重试，Terminal 仍仅有三种粗状态 |

“有 lease/有请求记录”不是进程存活证据；“缺记录”也不是提交从未发生的证明。
当前机制应保留的优点是未知副作用不自动重试，不能为解决卡住问题而删除这个保护。

## AUTH-01：授权等待及一次性消费

目标命令绑定 `command_id, conversation_id, run_id, wait_id, request_digest, expected_revision`；
授权 actor 必须由可信入口提供，不能接受模型自报。Grant 还绑定 Host/主体/资源/操作与寿命。
产品允许自由指令时，指令作为新的用户输入，不伪装成 Allow、不绕过重新评估。

1. Core 根据具体调用提出 Ask，Application 原子提交 wait、scope、请求与投影。
2. 客户端显示已提交 request；等待期间不调用模型轮询，不先执行工具。
3. 决议校验归属、digest、取消状态、policy revision 和委托权限上限。
4. decision receipt、once 消费绑定、Core 后继与 outbox 同事务提交；后续 Host 再核对 OS 可执行性。
5. 相同 command/payload 重发返回同一 receipt；相同 command 不同 payload 为 Conflict。
   已回答请求收到相同决议返回原事实，不同决议明确冲突，不能都返回 false。
6. once 绑定逻辑 effect，不因重放 ACK 二次消费、不对新 effect 生效。失败重试仍须核对权限有效性。

权限撤销与派发并发由事务顺序和执行前检查共同处理：撤销先于准入则不派发；
已开始的外部操作不能声称因撤销自动回滚，必须取消/核对并保留效果事实。
持久 grant 的来源和匹配规则必须可解释；禁止客户端用展示开关覆盖 Core 判定。

## AUTH-02：委托传输不是授权结果

```text
AwaitingDecision → 决议提交 → Claimed → TransportWritten
                                 └──断线──► OutcomeUnknown / Interrupted
```

具体状态以 Domain 的 delegated permission 定义为准。TransportWritten 只证明写入传输，
不证明接收方已执行、工具成功或授权可以对别的 occurrence 复用。
旧 generation/occurrence/digest 的结果不能批准新请求。重复帧只能重复 ACK，不生成第二授权。
断线后不能因为尚未收到成功而补发一次可能已生效的权限答复；根据协议可核对性决定恢复。

## REC-01：派发恢复决策表

| 已提交事实 | 必需检查 | 允许动作 |
|---|---|---|
| Prepared/未越过 start ACK | effect、attempt、归属与取消仍有效 | 有资格后 claim；不能绕过 start ACK 执行 |
| Leased，未开始，owner 死亡或当前 lease 过期 | 同 generation/token/revision；未被续租/替换 | 中断原 attempt，创建后继，再准入 |
| ExecutionStarted + Idempotent | 已声明的语义幂等性、预算、权限和取消 | Retry；幂等性由工具契约决定，不由命令文本猜测 |
| ExecutionStarted + Recoverable | 精确已接受 response cursor、可恢复协议 | Resume，不重新发起可能不同的请求 |
| ExecutionStarted + RequiresRecoveryDecision | 原始回执、资源观察、可核对外部操作身份 | OutcomeUnknown；禁止自动执行后继 |
| Settled | 提交身份与结果引用 | 读取/重放 ACK，不重执行 |
| 等待权限 | 有效 wait 与未决请求 | 恢复等待 UI，不把等待当故障重试 |
| canonical/checkpoint 身份不一致 | 原文、提交游标、协议版本 | 显式拒绝恢复，不能“修复”为成功/失败 |

OutcomeUnknown 必须进入 Core 的独立待核对义务，包含 effect/attempt 身份与原因；
当前仅 Preserve 的 Run 级设计要升级为可观察的 recovering/waiting 或 sealed(blocked)。
有安全 inspector 时执行核对；没有时受阻退出，并保留未确认副作用，不能永久 spinner。

核对结论至少分为 ConfirmedNotApplied、ObservedApplied(receipt/evidence)、StillUnknown。
ConfirmedNotApplied 才允许重新评估后创建执行后继；ObservedApplied 只补结果并继续语义核对；
StillUnknown 不授予重试权。用户说“再试一次”不是“之前没有执行”的证据。
当前仅 ConfirmedNotApplied 接口不足以承载以上全部出口；需与 COMPLETION 同批贯通。

## REC-02：取消及终态竞争

取消意图先持久化，再取消进程/连接；请求返回 CancellationCommitted 不等于已停止。
取消 scope 必须明确为 Run/Task 树，不能跨会话使用全局 stop 状态。
未派发 effects 可以撤销；已派发 effects 必须收集取消确认或未确认记录。
子 Task cancellation intent 持久传递，不以父 UI 消失作为取消事实。

Run 封存 cancelled 允许保留未知 effect 列表，含后续核对入口；不代表外部副作用全部终止或回滚。
未知 effect 核对是独立记录生命周期，不得因 Run 已封存而丢弃迟到回执，也不能改原 RunResult。
这需要新终态携带 unknown_effect_ids，不能仅将所有 ToolPart 一律改成“已取消”。
成功封存和取消争用 revision/CAS；后到操作返回已有事实。重启从同一事实恢复，禁止人为优先覆盖。

## REC-03：Supervisor 驱动与只读投影分离

Task 查询回答某个已提交 snapshot 中的事实；Supervisor 根据已提交资格尝试推进执行。
二者不得组合成隐式“刷新即执行”。`running_task_count`、`list_tasks`、`manage_runtime_tasks`
的 list/status、`pending_task_context` 原来会先 reconcile；本轮移除这些隐式入口。
`read_task` 与 `render_task_detail` 也不再顺带确认 UI 通知。Shell 等其他查询适配器仍需逐路审计。

当前明确的驱动点（不等于全部独立服务已实现）：

| 触发事实 | 当前代码锚点 | 权威限制 |
|---|---|---|
| TaskPromoted 的已提交准入 | core_agent_runtime / host_controls → reconcile_committed_agent_task | 核对 admitted authority，准备记录不足以启动 |
| 前台运行期间 Shell/Agent 释放槽 | background.rs / Agent reaper → reconcile_bound_supervised_tasks | 使用绑定的父配置；仍由 Store fair claim 决定 |
| 交互等待期间 task event 游标推进 | main_parts/interactive.rs → reconcile_supervised_tasks | 先快照游标再协调，不吞并发追加事件 |
| 显式恢复会话 | reconcile_for_resume_snapshot | 恢复是有副作用的用例；不能冒充 snapshot query |
| 精确授权回复提交 | reconcile_after_permission_resolution | 原 wait 归属、原请求、原准入和取消仍需核对 |
| 显式 wait_task | wait_task | 具有推进/等待语义，不承诺为只读；作用域校验仍须前置 |

`reconcile_for_resume_snapshot` 的旧注释称其“only projects external process facts”，
但其调用的 recover_orphaned_agent_tasks 可以启动 queued/retry_waiting worker，因此该描述不成立。
恢复命令必须在捕获最终 snapshot 前完成本轮已提交协调，并返回实际游标；查询接口不执行此步骤。

目标驱动信封（逻辑契约，尚未全部实现）：

```text
SupervisorWake {
  host_binding_id, generation, config_revision,
  reason: Startup | CommittedTaskEvent(cursor) | SlotReleased(attempt_id)
          | DeadlineReached(deadline_id) | PermissionCommitted(request_id),
  observed_at
}
SupervisorPass {
  inspected_through, committed_receipt_refs[],
  launched_attempt_ids[], pending_deadline?, observations[], errors[]
}
```

Wake 只是提示，不携带执行授权。每次派发从 Store 重新获取资格、取消状态与 fair slot；
重复 Wake 不创建第二 attempt。SlotReleased 不能仅凭进程退出就宣布业务成功；以真实回执协调。
同一 Host 的并发 wake 合并为串行 pass；多 Host 仍由原子 claim/generation 防止重复执行。
单次 pass 必须有有界读取量、工作量与下一游标，禁止随每个 token 全表扫一遍。

重试到期/租约过期即使没有新的 task event，也必须有显式 deadline wake；仅游标变化不足以保证活性。
无窗口、输入框隐藏、父进程在模型请求中也必须由实际存在的 driver 维持推进。
目前未证明所有运行形态都具备这一条件，不能仅凭交互 loop 和释放槽回调宣称独立 supervisor 完成。
部署必须明确 driver 是服务进程还是运行期 Host，以及 owner 死亡后由谁重新启动；无人驱动时显示
可恢复的未执行状态，不谎报正在执行。退出清理本地句柄不等于取消 durable Task。

Store 错误必须进入 SupervisorPass.errors 并保留未消费 cursor；不能把 `.ok()`/空列表当“没有任务”。
允许有界退避重新读取；不能对 unknown effect 擅自重试。配置/权限失效为具体阻塞而非无限定时唤醒。
P03/P07 分别计 queue→claim、claim→start ACK、deadline→pass、pass 扫描数与提交次数；
baseline 需区分单会话/多会话及冷启动，无实测前不填写毫秒承诺。

验收同时要求：查询无新 attempt/worker/ACK；显式准入能启动；槽释放自动推进；无新事件的到期恢复；
重复/并发 wake 单次副作用；无 UI 冷重启；Store 读取失败不丢 pending wake。当前仅部分已有测试。

## 验收映射及缺口

现有可复用测试（本轮运行结果及其范围见 VERIFICATION 的执行记录）：

- Kernel `effect_delivery_attempt`：`settled_and_outcome_unknown_attempts_cannot_create_recovery_successors`。
- Kernel `delegated_permission`：`recovery_never_converts_a_claim_to_safe_retry_or_denial`。
- Application `run_recovery_port_tests`：`effect_delivery_recovery_plans_only_evidence_safe_transitions`、
  `expired_recovery_mixed_deliveries_never_treats_expiry_as_whole_run_death`。
- 同文件 `orphaned_permission_delivery_uses_transport_evidence_without_retrying_unknown_writes`。

Application 的 MemoryMaintenance fixture 只能证明恢复计划，不证明 SQLite 原子性、真实进程死亡和各端 UI。
E2E-05/07/09/12 需补：双击/冲突批准、撤权竞态、父取消树、kill 在 start ACK 前后、
副作用成功但回执丢失、核对已生效/未生效/未知三分支、重连乱序与旧 owner 回执。
每例同时断言实际副作用次数、已提交 wait/attempt/result/seal、父子任务状态和 UI 对应会话。

性能采用 P03/P07：将人工等待独立计时，分别记录核对耗时、恢复读取量、重放事件数；
不能把等待用户 10 分钟算作工具执行 10 分钟，也不能把恢复策略 Preserve 当作恢复成功。

## 原生未知效果的用户决定出口

### iOS A08 owner 接线（源码完成，尚未运行）

`IOSUnknownEffectRecovery` 定义当前协议 49 的检查、固定决定和回执 DTO；
`IOSApplicationRuntime` 继续使用唯一 pending-command 文件，不增加恢复日志。
用户确认时固定 resolution/time/request ID，文件保存完整业务 input；不确定恢复只读取并
重放该 envelope，核对 reply 的 Run/resolution/choice 后才删除。启动时保留 A08 pending
供对应会话 owner 显式消费，不让通用启动重放吞掉其待处理结果。

恢复先查询原 command receipt，只有明确 null 才重发同 ID 原 envelope；查询失败不是
“未提交”。FFI 外层 ok=false 即使标记 nonrecoverable，也不是 canonical rejected receipt，
必须保留意图。回执 decoder 在清理前核对 client/protocol/request、完整终局字段及状态形状，
坏 conflict/rejected/result 不能删除 pending；A08 另核业务 reply 的原 Run/resolution/choice。

`ChatManager` 每会话 execution 持有冻结检查，公开无租约 token 的 presentation；每次
检查分配新的 UI inspection identity，因此即使可见 attempt ID 相同，旧确认闭包也不能
批准新 revision。`inspectUnknownEffects`、`confirmUnknownEffectRecovery`、
`recoverPendingUnknownEffectRecovery` 均要求显式 conversation/Run/assistant，根 manager
只分派原会话 owner，不使用点击时选中会话替换身份。远端 mirror 不可本地决定。

retry_authorized 仅进入既有 `runDeviceAgentTurn(recovering:)` → `recoverExactRun`；
unknown_preserved_blocked 必须读取并匹配原 terminal seal 的 Blocked 结果，不能自行
置成功或发送新消息。独立 pending 工作不能通过 block 隐式取消。源码测试覆盖固定业务
时间/身份 roundtrip、原 pending envelope、错误 owner reply 与独立工作拒绝。

`UnknownEffectRecoverySurface/Card` 已接 ChatView 的恢复区域。启动、选会话和失败后只做
后台发现，内联呈现风险，不自动弹窗；用户点具体重试选项才出现系统 confirmationDialog，
包含 Cancel 且无默认确认快捷键。保留受阻也是明确选择；pending 只显示检查原决定，
不让用户重复授权。切会话/检查身份变化立即关闭旧确认；Manager 再核冻结 presentation。
使用系统语义颜色、Dynamic Type、44pt 最小点击区域与纵向布局，不增加装饰动画。
中英文文案已补，其它语言暂采用英文回退。

发现其它会话的 pending 意图时只提供“打开对应会话”，不强制切换当前会话、不自动恢复。
会话及设置投影通过协议 49 的保守显式查询集合绕过 mutation journal（对应
`command_effect.rs`，不按 get/load 前缀或 payload 文本猜测）；未知命令继续受原 pending
写入顺序约束。源码回归保存真实 pending 决定后读取设置、断言原意图不变且新写被阻止。

交互依据 apple-design：`alerts.md › Best practices` 要求启动问题以内联可发现方式呈现；
`action-sheets.md › Best practices` 使用用户主动发起的确认与 Cancel；
`accessibility.md › Mobility` 与 `typography.md › Supporting Dynamic Type` 对应44pt点击区域和
不截断的纵向系统字体布局。仅源码审查，未测量设备实际对比度或无障碍布局。

新路径的 pending 读取、检查、决定提交/回执查询、exact invocation 与 seal 读取通过
后台同步任务执行，每个原文件锁/FFI 事务在单一同步作用域结束，锁不跨 await。已有
会话元数据查询与模型恢复准备仍沿旧 MainActor 入口，尚需统一性能观测，不能称整个 iOS
Application 已全面脱离主线程。尚未运行故障注入、真机、VoiceOver/Dynamic Type、恢复后
流式呈现及进程在决定提交后/公共恢复前退出的验收，不宣称真实 UI 闭环已验收。

后端协议 49 提供 `InspectUnknownEffectRecovery` 与 `ResolveUnknownEffectRecovery`；原生
Mac/iOS 已接用户决定控件与原运行恢复链源码；添加 typed adapter 不是闭环验收，
必须让用户基于原运行的实际未知尝试作决定，并观察已提交回执。不能发新模型消息代替恢复。

接线沿现有 owner，不建立第二个执行器：Mac 在 `MacWorkspaceModel.checkLocalRunRecovery` 的
`localRunRecovery`（conversation、expected Run、canonical assistant、generation）上附加检查状态，
由 `MacUnknownEffectRecoveryControl` 经同一 ApplicationService 调用原命令。iOS 使用
`ChatManager` 每会话 execution 实例的 `IOSRecoveryObservationFence`、canonical active Run 与
`IOSCanonicalAgentApplication`，扩展目前只识别 permission wait 的冷恢复呈现。切换会话只改变
观察者；确认闭包捕获原 owner，不能读取点击时选中的新会话来授权。

检查输入固定为 conversation_id、expected_run_id、assistant_message_id。返回的 Run、assistant
身份必须一致；完整 targets（effect/request/attempt、worker generation、lease owner/token、attempt
revision）和 invocation digest 私有保留，不显示 fencing token、不写诊断日志，也不交给模型。
界面展示具体操作与“先前可能已经生效”的风险，用户可以暂不处理，或明确选择：

- `AuthorizeRiskRetry`：携检查得到的单个完整 target 与非空理由。回执仅表示允许风险重试，
  不表示已执行或原操作未生效；随后通过同一 exactRun 公共恢复入口派发。
- `KeepUnknownBlocked`：携全部未知 targets 与理由。存在 independent pending effects 时不能
  封存，先让独立工作收敛；成功后查询原 Run canonical seal，再展示 Blocked，不设置成功。

用户确认时一次生成并冻结 resolution_id、resolved_at_ms 与完整命令。传输不确定后查询原
command receipt，或以原字节与同一身份重放；不能更换时间或身份绕过冲突。iOS 复用
`IOSApplicationRuntime` pending-command 文件；Mac 使用每 Run 私有有界意图文件，恢复时先查询原
回执，不以传输错误退休意图。两个客户端都应核 reply 的 resolution_id/run_id，
Conflict/Rejected 重新检查，不自动重新授权。`VerifyAppliedReceipt` 不向普通 UI 暴露手填结果，
它只能核验既有原 Host receipt。

远端运行的检查与决定必须送到真实执行 Host，不能在只读镜像库依据“没有 attempts”判断无风险。
Mac 的 inline 控件只显示在检查确认的本地 owner 上；每次检查更换展示快照以清除旧选择，
风险确认无默认回车按钮，可取消，启动不弹窗。确认命令回传冻结快照，WorkspaceModel 再核
conversation/Run/generation 与 target 集合；RetryAuthorized 接公共 check/resume，Blocked 仅
读取 canonical seal。源码回归覆盖真实 WorkspaceModel 准入、检查、确认、同 Run 恢复，以及切换
会话后旧确认不送达；A08 服务边界在这些 UI 回归中显式脚本化，不能代替 Store 副作用验收。
本轮尚未运行编译、测试及 UI 检查，不能声称原生实测闭环。远端 Host 决定出口仍待接。
验收须覆盖确认后断连/重启、会话
切换后迟到点击、attempt revision 变化、重复提交、独立 pending 工作及真正副作用重复风险。

### iOS 待确认命令文件的持久边界

`IOSPendingCommandJournal` 只保存一个完整原 Application envelope，不记录业务结果，不赋予
重试副作用的权限。Runtime 保留自身句柄锁，再取得 journal transaction；锁覆盖读取、回执
查询或原 envelope 发送、核验回执、保存全过程。文件锁跨实例，不能将独立 load/save 上锁
误称为提交原子性。作用域同步执行，无 await，退出释放锁；普通只读查询不建立待确认意图。

单 envelope 的实际 sortedKeys JSON 不超过协议 `APPLICATION_SERVICE_MAX_FRAME_BYTES`
的 16 MiB；map 至多一项，key 必须等于 envelope 内 request_id。转义后的 key 已包含在原
frame 中，所以完整 map 的上界为 32 MiB + 16 字节；不是单独按字符串长度估算，更不截断正文。
读取同时检查 fstat 大小和实际读取字节数，null、空文件、错误结构或身份不符均拒绝。

同一数据库目录 FD 固定后使用 openat/renameat，文件与锁必须属于当前用户、为单链接普通文件、
权限 0600，末级链接不跟随；不更改用户目录权限。临时文件私有写入，先 fsync 文件，原子 rename，
再 fsync 目录。任何写入持久确认失败使当前 journal 实例停止后续提交，不得发送或替换原意图。
新实例在读取前重新同步现有文件及目录，只有成功后才解释可见状态；磁盘持续同步失败继续拒绝。
唯一已退休状态是显式空 map，由 Runtime 在核验同身份终态回执后写入，不用删除文件或 null
表示退休。不存在的文件仅代表尚无可读日志，不能据此证明任何历史执行成功；存储损坏或外部删除
无法由客户端意图文件恢复，仍以 Application receipt/执行事实为权威。

源码覆盖完整 envelope 重开、接近协议上限、错误身份/超额拒绝、私有文件/链接/损坏拒绝以及
两实例 load→send→save 排他边界。本轮未运行编译或测试，不声称已验证断电与真实设备持久性。

### Android 未知效果恢复出口

Android 使用同一 `InspectUnknownEffectRecovery` / `ResolveUnknownEffectRecovery`，typed DTO
只映射共享协议，不定义 Android 业务结果。`AndroidApplicationRuntime` 的普通 pending journal
保存完整决定 envelope，启动恢复不提前消费 A08 决定；用户回到原会话后先查询同身份回执，
仅不存在回执才重放原 envelope。非可信传输错误不能构造 Rejected 或删除决定。

`ChatViewModel` 绑定原 conversation、Run、load generation 与独立展示快照。旧会话点击和旧检查
确认被拒绝；正文中 inline 状态展示未知操作，Compose 风险确认由点击触发，可取消，不在启动
弹窗。只显示 effect ID，不显示或日志输出 lease、worker、token 等 fence。`AwaitingRecovery`
是客户端等待呈现（不是 Run 业务状态），阻止重复提交而不显示生成动画。

`AndroidAgentTurnService` 先核 canonical 本地 owner，远端镜像不能决定；RetryAuthorized
进入统一 `recoverExistingRun`，沿原配置/模型凭据证明、Application 工作调度、原子 claim 和
既有 Host loop 推进，不调用 Begin。入口拒仍有未知 targets；过期 lease 的处置仍由 Application
恢复事务决定，不因 worker 过期自动重发未知写入。普通 permission 冷恢复复用同一入口。
KeepUnknownBlocked 只提交全部 targets 且 independent pending 为零；收到回执后验证原
Run canonical seal 的 Blocked 结果，再加载投影，不由 UI 设置成功。

源码包含 DTO/回执 identity、单 target/完整阻塞集合、旧会话 generation 拒绝和等待呈现反例，
现有 native permission recovery 回归改用公共入口。尚未编译、运行 JVM/设备/Compose 测试；
真实断连重启、风险副作用计数与跨会话点击仍必须进入最终统一 Android 验收。

## 到期恢复决策与取消专用路径（2026-10-28源码核对）

恢复分为“观察原物理所有权”“规划delivery恢复”“让Core重绑定可Resume的effect”“实际派发”四步，不能把其中一步的成功当作后续完成。Task lease决定task worker可否接手，effect lease决定具体外部操作的派发权，两者有独立到期时间。

| 已观察事实 | 合法处理 | 禁止推断 |
| --- | --- | --- |
| delivery仍有效 | 不抢占；等原owner续租/返回或到期 | App进程重开不证明旧worker已死 |
| attempt已terminal | 保留不可变证据，重复启动只读 | 不能重新派发来“确认一次” |
| 到期且Core恢复计划为Resume | 校验active Run、冻结invocation digest及pending outbox；按原effect生成ResumeHostEffect | 不重放原请求，不为unknown outcome自动Retry |
| 原pending权限需要重示 | 恢复事务核对原outbox/witness/pending请求，产生Core展示trace | 不制造Allow、不沿用旧投递owner回答 |
| target已Cancelling | 保留原target与精确cancel control，推进取消核对 | 不因target退出普通可执行outbox就把物理效果当成消失 |

`PrepareRunEffectRecovery`按64条分页扫描interrupted候选，只选择recovery_plan=Resume；effect须仍属原Run且pending，request不再匹配的记录不会被当作当前Resume。确定transition_id为 `resume-effect:{attempt.id}`。返回rebound_count/traces只表示Core准备提交，不表示工具已重跑或业务完成。

移动child准入首先核对不可变Task admission与已有child binding；若原child已terminal，按原终态结算而不是建新Run。正常执行才选择fresh工具观测、校验冻结配置、恢复过期task lease并公平claim。有效前任lease返回无准入，不与其并行运行。

cancel_requested且已有binding时另走cancellation_only：有效前任lease仍等待；到期后必须存在同bound active Run和原assistant，使用原取消时间提交持久取消。沿原attempt作为身份见证返回，不续普通执行lease，不创建新模型/工具工作。未bound的取消任务不会借此创建child。最终Task回执从bound Run terminal核对，父Run answered不替代child结果。

故障反例：审批界面退出、协程CancellationException或流onTermination只能detach；若将其转换成业务Interrupt，会把父取消级联给本应继续的child。恢复时禁止清除cancel_requested、换task ID、换DB或把原失败抹成新任务成功。
