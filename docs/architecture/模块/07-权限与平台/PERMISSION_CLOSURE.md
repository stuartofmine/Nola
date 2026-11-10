> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 权限请求决定与生命周期关闭

## 授权范围所有权：现状审计与替换契约（2026-10-12）

批准分支 fixture：standalone_approved_permission_revokes_grant_after_tool_result_and_terminal
覆盖 AllowConversation 派生授权存在、Core 实际派发 ExecuteTool、模拟 Host 返回
Succeeded/Failed 后终态对账。故障注入时授权撤销与 Task receipt/请求关闭一同回滚；
成功提交及冷重开后 grant 保留 revoked_at_ms，决定/来源不变。模型/工具结果均为
fixture，不把无 Failed seal 等价于 ActionCompleted，不证明真实文件效果。

生命周期补充验收：standalone_denied_permission_closes_on_model_completion_and_failure
从实际 Core 权限 effect 独立 Ensure（全程无 suspension）开始，用户拒绝后分别
提交模型正常结束和非 retryable Failed，实际 reducer 生成终态并由原 binding 对账。
关闭 UPDATE 故障必须同时回滚 Task receipt；冷重开后 withdrawn 保留原拒绝及时间，
closure 引用原 seal，Ensure/终态重复对账不复活请求或新建 Run。此处使用模型 fixture；
批准后真实工具成功/失败与 grants 撤销组合，以及真实客户端显示仍需单独验收。

本节是待实现的整改契约，不代表现有 scope_rule 已安全闭合。请求来自真实 Core
effect，与请求所携带的可复用授权范围已获证明，是两个不同命题。

第一阶段实现：`nola_domain::permission_scope` 已提供 ConcretePermissionSubject、
PermissionResource、CapabilityScope、ResourceScope、ReusablePermissionScope 与纯
evaluate。它区分 Covered/OutsideScope 与结构错误；文件按组件/文件系统/目标/access
匹配，多资源必须全部覆盖，能力集合必须显式且不可重复，网络要求规范 origin，
进程仅匹配完整调用摘要。无文件系统 IO、无提示词解析、无授权写入。
当前还未接入 Core effect/Store/客户端，不是旧 scope_rule 的运行时兼容分支。
原 PermissionSubject 仍服务旧策略评估，待纵向切换后合并/删除相关旧入口；不能
同时保留两个有效授权判断。来源核验接线与执行前身份复核尚未实现。
本模块只回答范围覆盖，不证明用户曾选择该范围或资源解析证据当前仍有效。

第二阶段纯契约：FrozenPermissionOffers 固定 occurrence + 完整 subject + offers。
freeze 自动包含一次性选择，拒绝重复候选及不能覆盖原 subject 的可复用范围；
offer_id 是带域分隔的 typed 内容摘要，绑定原 occurrence、subject、具体 scope。
PermissionOfferSelection 只接受 occurrence/offer_id，禁止携带替代 scope；select
重新校验集合完整性后借用原候选，不从客户端输入重建范围。输入摘要、effect、范围
变化都会使旧选择失效。摘要只是完整性检查，不是签名；Store 必须从已提交 Core
effect 读取集合，不能信任客户端提供一份自行重算摘要的集合。当前仍未接入生产。

Core 接线审计：`runtime/tool_driver.rs` 创建 PendingPermissionRequest 与 AgentEffect
时目前只有旧字符串 subject/resource；`runtime/effect_dispatch.rs` 将其映射为
HostRequest::RequestPermission，仍没有具体资源集合。应先让工具权限适配契约产生
ConcretePermissionSubject，再在获得原 effect identity 的提交路径冻结 offers；
不能由 Store 将字符串包装为“已验证资源”来跳过解析和身份边界。所有恢复路径
必须读同一已提交集合，不在重发/恢复时重新派生候选。

已核对的原实际链路（下方列明本轮收紧，未完成整体替换）：

1. `cli/src/task_runtime_parts/supervision.rs` 从 capability/input 创建 ToolRequest，
   调用 `tools::session_permission_rule` 得到字符串，写入 StoredPermissionRequest。
2. `cli/src/tools/parts/permissions_output.rs` 为文件操作选择父目录，并可能通过
   `nearest_permission_project_root` 扩大到项目根；浏览器资源统一为 browser。
   这些派生会依赖当前文件系统状态，不是原 Core effect 的冻结事实。
3. `permission_source::capture` 比对 capability/input/指导及执行身份，未比对
   scope_rule 与原 RequestPermission.resource/evaluation。
4. Application 从已存 scope_rule 构造 `cli::tool::allow_rule` grant；Store 只证明
   grant 与这个字符串相等。相等证明未被修改，不证明范围最初合法。
5. CLI 读取 task Host scope 的 grant，用自己的路径前缀/命令范围匹配函数决定
   是否直接返回 AllowTask。JSON 中的非 bash `tool` 字段没有被 matcher 检查。
   当前可能有意授权同类工具，但 schema 未明确“精确工具”与“能力族”的区别。

因此不能在 CLI 多加一个 tool 字符串比较就宣布修复：那会改变既有范围语义，
仍留下 Core/Store/Host 分别解释授权范围、文件系统改变后范围漂移的问题。

生产侧收紧已实现：scope_rule 格式 3 强制精确 tool；格式 2 不兼容复用，不将
原能力族授权静默解释为精确授权。移除 nearest_permission_project_root，目录
标记不再扩大范围；缺失/空 resource 和缺少原命令不能成为匹配通配符。修改了
原来要求跨工具、项目根复用的测试语义，新增畸形规则测试。现存文件目录复用、
命令族复用并未被这些检查证明安全；本节 typed 资源解析和原 effect 冻结仍待接线。

### 模块所有权与数据边界

Kernel 解释契约：配置 pattern 和交互批准产生的资源不是同一种语言。
Conversation 的 capability 按字面匹配；资源优先按字面相等，只有既有
显式目录/命令范围进入其解析器，其他内容不执行 glob。工具输入中的 `*`、JSON
文本及未知能力名不能自行创建通配授权。ProductDefault/UserGlobal/Host 的配置
规则及 Turn 的 /allow 显式策略仍可使用 pattern；Turn 仍受 live origin 准入
约束，不得带入后台。shell 编码范围只由对应 shell 能力解释。这一收紧
不证明来源字段本身可信，也不证明目录/命令范围已完成 typed 资源身份验证。

| 当前规则来源 | 内容语义 | 生效前置 |
|---|---|---|
| ProductDefault | 产品默认 pattern；敏感操作仍受 profile guard 约束 | 当前执行配置，由默认策略构造 |
| UserGlobal | 用户显式配置 pattern | 配置准入校验，不能由模型工具结果创建 |
| Host | Host 配置策略 pattern | 受信配置边界，不以返回 JSON 的 source 自报建立权威 |
| Conversation | 已批准调用的能力／资源数据，或 Core 生成的明确范围 | Core 原请求、用户决定及来源证据；非 live origin 还需显式允许继承 |
| Turn | 当前输入的显式策略（包括 /allow），可含通配 | live user authority；后台输入不得继承 |

这是现有字段的精确定义，不是用 source enum 代替来源证明。后续 typed offers
接线须保持这些区别，不能因“都是交互授权”将 /allow 降为某个调用的字面授权。

已关闭 Host 覆盖入口：tool_driver 的普通/委派会话授权仅保留原 Core resource，
不读取 grant 字符串或 resource_pattern 作为替代范围；grant 中的执行数据不具备
范围声明权。CLI 前台不再传入自派生目录，终端提示保留具体资源。需要更宽范围时
必须接线以下冻结 offers，而不是恢复任意 JSON 覆盖。native/custom resolver
尚未接收统一冻结显示数据，独立 task Host grant matcher 也尚未移除。

| 模块 | 拥有的事实／输出 | 不得承担的责任 |
|---|---|---|
| Domain | 严格类型化 PermissionSubject、GrantScope、ScopeOffer 与匹配结果 | 不读取文件系统，不根据自然语言选择工具 |
| 工具适配器 | 根据工具 schema 给出操作、资源参数及所需权限；解析真实资源的证据 | 不凭目录标记扩大授权，不直接持久化用户决定 |
| Core | 将具体 subject 和可选 scope offers 固定在 RequestPermission effect；执行前统一评估 | 不从 UI 显示文本恢复规则，不把缺证据解释成允许 |
| Store | 核验 committed effect、保存原 subject/offers；原子选择 offer、写决定与 grant | 不接受客户端自定义 resource_pattern，不重新运行 CLI 派生逻辑 |
| Client/Host | 展示冻结范围、提交 request_id + offer_id + 用户决定；执行资源校验 | 不维护第二套匹配器，不修改范围、不替用户扩大选择 |

具体类型约束（实现时删除旧字符串规则与兼容分支）：

- PermissionSubject：原 capability identity、operation、执行目标 identity、规范化
  resource set、原输入摘要、解析证据引用。多资源操作必须保留全部资源及各自的
  access（例如 move 的源与目标），不能挑一个 destination 代替整个操作。
- GrantScope：显式区分 ExactInvocation、FileResource、DirectoryTree、NetworkOrigin、
  ProcessInvocation、PlatformResource；能力选择器明确 ExactCapability 或声明的
  CapabilitySet。不得用空字符串、通配工具名或 display label 隐含“全部允许”。
- ScopeOffer：offer_id、scope、subject_digest、effect occurrence、显示信息的结构化
  来源。Store 从原 effect 读取 offer，客户端只提交 offer_id，不提交替换 scope。
  offer 必须覆盖原 subject，不能因可授权范围不匹配而创建无效或更宽 grant。
- PermissionDecision：AllowOnce 精确绑定原 occurrence；AllowScope 必须引用用户
  实际选中的 offer；Deny 与关闭保持独立。未展示/未选择 DirectoryTree 时，不得
  因存在 Cargo.toml、.git 等标记隐式授予整个项目权限。
- DerivedGrant：原 request/offer/effect、所选 typed scope、所有者 Task/Run、用户
  决定时间与撤销状态。scope payload 只保存一次；UI 文案和 CLI 摘要均为投影。

未知工具通过自己的 schema/权限适配器声明 subject；未提供可复用范围的工具仍
可请求 AllowOnce，不得猜测它属于文件、浏览器或 shell。增加新资源种类需要
契约实现和测试，不靠用户提示词关键词或语言特判扩充路由。

### 资源语义与执行流程

文件范围必须包含执行环境/文件系统身份；相同路径字符串在远端与本地不是相同
资源。路径解析在 Host 资源边界进行，保留基础目录和解析证据，处理不存在目标、
符号链接及父目录变化。DirectoryTree 匹配路径组件而非字符串前缀；执行前资源
identity 变化必须重新评估或重新请求，不静默扩大到解析后的任意目标。项目根
可以作为显式候选展示，但不是自动派生的授权事实。

网络 origin 以结构化 scheme/host/有效端口匹配，重定向后的目标另行评估；browser
不是全站网络授权的资源名。ProcessInvocation 固定执行环境、cwd、程序/argv 或
shell 脚本身份；命令族/前缀授权只有在存在明确定义、可验证的专用 offer 时才可
出现，不由两个 shell 字符串恰好有共同前缀推导权限。

正常链路：资源解析 → Core 提交冻结 subject/offers → Store 核验并保存请求 →
用户选择 offer → 同事务提交决定与 derived grant → Core 用同一 matcher 对具体
effect 评估 → Host 执行前核验资源身份 → 提交真实执行结果。授权仅允许执行，
不是成功结果。后续同 scope 的请求复用 grant 也必须经过 Core 评估，不能由
background wait 的 CLI 分支自行返回 AllowTask 绕过这一步。

失败分类与恢复：

- 缺失/不匹配的来源或 offer：EvidenceMissing/Conflict，不创建 grant，不重试模型。
- 资源发生变化：ResourceChanged；保留原决定历史，对新 subject 重新评估。
- 用户拒绝：Denied；取消/终态：Closed；连接丢失：DeliveryUncertain。彼此不得合并。
- 重启：从持久 subject/offer/grant 恢复，不能重跑项目根探测改变原授权范围。
- 重放：同 request/offer/决定返回原回执；异 offer 或异载荷冲突，不扩大授权。
  关闭后的旧选择返回 Closed，不复活 grant；事务故障不留下决定或授权的半成品。

### 替换顺序与纵向验收

先建立 Domain 类型和唯一 matcher，再扩展 Core effect 的冻结 subject/offers；
接入 Store 核验与事务后替换 CLI/Mac/Web/mobile 的展示与选择，最后删除
scope_rule、cli::tool::allow_rule 及 Host 自行匹配。必须完整切换一个纵向入口
再验收，不同时保留两套解释作为“兼容”。当前 schema 72 尚未进行此替换。

验收必须覆盖：同目录不同文件、读/写/删除不同操作、跨项目、路径 ..、符号链接
变更、目标尚不存在、多资源移动、相同路径的不同执行环境、网络端口/重定向、
shell 复合命令、未知工具、显式范围扩大、撤销与复用并发、冷重启、事务故障。
至少一个真实用户选择范围→后续匹配调用无需重复询问→范围外调用重新询问→
实际落库的全链路；同时验证自然追问、多语言和模糊措辞不改变具体工具权限契约。

测量范围派生、Core 评估、Store 提交、弹窗出现与确认后恢复的分段耗时；分别
记录冷/热运行与资源数量，并统计候选数和拒绝原因。先记录真实基线再设预算；
不得通过省略资源校验提高“自动批准率”，也不能把模型耗时混入权限评估耗时。

## 请求关闭的当前实现

### 资源解析接线协议（待实现，不是现有 Host effect）

响应关联纯契约已实现：PermissionResolutionRequest 固定 resolution_id、执行目标、
由规范化输入所有者建立的 invocation_digest、capability/operation 和完整 demands。
PermissionResolutionFacts 的 bindings 必须逐项包含原 demand、resolved resource、
Host evidence_id。subject_for 验证请求身份/输入摘要/目标、需求完整性、kind/access、
非空证据引用及资源结构；按原需求顺序形成 ConcretePermissionSubject。响应可乱序，
不允许缺项/额外项/重复 occurrence。原 bindings 全量保留，匹配 subject 仅去重
完全相同的 resource+access，不把“相同路径的两个需求”从证据里删除。
此方法不认证 Host、不读取证据、不验证其当前有效性；不能在未核对原 Core effect
delivery 的情况下接受客户端构造的 facts。invocation_digest 必须来自已提交规范化
输入事实，不能采用模型自报摘要。生产归约与目标 Host 的证据校验尚未接线。

现有 `cli/src/tools/parts/resource_evidence.rs::resource_content_version` 会读取文件
内容生成版本，validation_snapshot 还会遍历目录。它们服务执行/验证证据，不能
直接复用为授权前资源解析，否则会在授权前完成受保护的读取。新解析只能收集
允许范围内的身份元数据，资源内容读取仍必须等待执行授权。

需求提取纯契约已实现于 `nola_domain::permission_resource_plan`：
PermissionResourcePlan 包含 capability/operation 和具名 ResourceRequirement；
ResourceSelector::One 使用 JSON Pointer，Each 指定数组及每项内的指针。所有声明
与所有元素都必须存在，缺项/空数组/错误类型分别返回结构化错误，不输出部分
成功；结果保留 requirement_id/item_index/access/locator_kind 及原始 locator。
相同路径的不同需求和数组出现位置不去重（源与目标、不同 access 不能合并）。
这里只产生 UnresolvedResourceDemand，没有执行目标资源身份证明，也没有授权。
Host/InputNormalization 生产声明仍使用旧类型，尚未切换到此计划。

提取不 trim 路径、不展开 ~/cwd、不查项目根。locator 仅拒绝空值/NUL，空白、
反斜杠和换行等字面值保留给目标解析器；平台合法性不能由本地通用字符串规则
代替。Domain matcher 接受目标解析器产生的 POSIX 反斜杠/换行文件名组件，不把
它们当作跨平台分隔符；界面必须转义显示，不能原样当成控制序列或日志行。

补充代码审计：`HostInputNormalization.permission_resource` 已有声明，定义在
`core/src/agent/host.rs`，不是缺少所有目录元数据。其 resolve 是纯字符串提取：
FirstOf 取第一个存在的字段；Composite 原来用 filter_map 忽略缺失字段。
现已改为全部字段存在且非 null 才投影，否则保留 FullInput；FirstOf 不再 trim
合法路径。内置移动/复制/批量文件/补丁已共用 FullInput 声明，保留完整调用输入。
这只是防止信息丢失，尚未提供每资源 access 或 Host 身份证据。
`HostCapabilityDescriptor::permission_subject` 调用该 resolve，tool_driver 将结果
固定到工具状态。另有 permissions::permission_resource_for_input 及 CLI
session_permission_resource 两套派生。后续接线应替换这组声明及消费，而非在外面
增加第四套 capability 名称判断。

具体缺口：built-in move_file/copy_file 的声明来自 FirstOf(destination,path,...,
source)，不表达源与目标各自权限；read_many_files 的 path/paths 也是候选字段，
没有“数组中每项都必须覆盖”的明确规则。文件身份、基础目录、执行环境、软链接
处理均未包含在该字符串契约中。这里证明信息丢失，不等于已证明真实越权执行；
推荐策略、执行沙盒等其他边界仍可能拒绝，必须分别验收。

不能直接把这段字符串包装成 ConcretePermissionSubject。规范接线需要一个持久
的资源解析步骤，以下名称用于契约设计，尚未加入 AgentEffect/HostRequest：

| 交互 | 必填输入 | 输出与校验 |
|---|---|---|
| ResolvePermissionResources | 原 Run/call/input 摘要、目录能力 identity/revision、执行目标 identity、规范化参数、资源需求声明 | 解析状态与 resource bindings；不得返回用户允许/拒绝 |
| ResourceResolution | 同一 effect/request identity、完整需求覆盖、解析证据引用、ConcretePermissionSubject | Core 核对原输入、目标、能力和每个需求；缺项不能成为空集合 |
| RequestPermission | 已提交解析结果引用、原 occurrence、冻结 offers | Host/Client 只展示和选择，不重跑解析改变候选 |
| ExecuteTool | 原规范化输入、选中 grant/offer 引用、资源证据及同一执行目标 | Host 原子使用/复核资源身份；变化返回 ResourceChanged，不静默改目标 |

工具资源声明必须直接属于 versioned capability contract，表达参数选择器、
单值/数组/全部元素、每个资源的 access、操作 identity、允许的规范化与显式默认。
例如 move 必须有 source 的移除权限和 destination 的写入/覆盖权限；copy 的 source
只需读取，不能因同为“两个路径”而合并策略。schema 已完成参数别名归一化后才
提取需求，不再用 destination/path/root/cwd/source 的优先列表猜测语义。
未知第三方工具无此声明时，绑定完整 invocation 的一次性批准仍可存在，但不得
声称已有可复用目录/origin 范围。它应由工具适配器声明自己的资源语义，不由工具
名字或用户措辞推断。

解析步骤的能力限制：它只在已经配置的执行目标上解析本次调用所需的资源身份，
不读取文件内容、不遍历目录内容、不执行 shell、不连接待授权网络目标。元数据
解析同样受 OS/沙盒约束；若需要额外访问能力，则返回 ResourceAccessRequired，
等待用户完成该能力授权后重试同一个解析意图。不能为“准备权限弹窗”先执行工具，
也不能自动获取越界目录权或把 OS 拒绝当作解析成功。

纯 URL origin 规范化不需要网络；文件解析由目标 Host 提供文件系统/基础目录身份。
不存在的写入目标保留已解析父目录身份和叶子名；不能用字符串 canonicalize 失败
后回退到未经解析的绝对路径。远端 Host 的路径必须在远端解析，不能由本地路径
库解释。持有资源句柄能避免竞态时，应让执行使用同一句柄；句柄不可持久化，
重启后只恢复证据引用并重新核验，不能反序列化一个数字就认定仍持有原资源。

状态与结果必须区分：

1. PendingResourceResolution：尚无足够资源事实，不能提前显示范围批准选项。
2. ResourceAccessRequired：解析被 OS/沙盒权限阻挡，等待外部授权；不是用户已拒绝
   本次工具，不提交伪造 deny，也不要求模型换一种命令绕过。
3. AwaitingPermission：解析已提交且 offers 已冻结，等待精确用户选择。
4. ReadyToExecute：Core 已选中有效授权，Host 执行前仍需核验资源身份。
5. ResourceChanged：曾解析身份失效，保留原决定与执行历史；新解析生成新 subject
   和候选，旧 offer_id 不能批准它。对仍被既有 typed grant 覆盖的对象可重新评估，
   但不能跳过身份校验。
6. Failed/Cancelled/Closed：分别记录真实解析/执行失败、取消来源及请求关闭，不能
   都转换为 validation_required。解析成功本身不代表工具或用户任务完成。

持久顺序：Core 提交解析 effect→Host 返回资源事实→同一原响应归约事务保存解析
事实与下一权限 effect→Store 创建权限请求及来源→原子提交用户决定/grant→Core
选择执行。解析响应按原 effect/attempt/序列幂等；冲突载荷拒绝；响应已提交但
回执丢失时重放原结果，不能重新猜资源或生成新权限卡。取消与解析响应竞争时由
同一 Core 归约顺序仲裁，迟到事实不创建新的待授权请求。

完整切换的验收门禁：实际目录声明→规范化输入→资源解析→权限候选→用户选择→
执行前复核→真实工具结果→终态落库；分别覆盖冷重启、解析中取消、OS 访问受阻、
父目录/链接替换、源目标部分覆盖、空数组、缺字段、同名本地/远端路径。不得仅用
手工构造的 ConcretePermissionSubject matcher 测试证明此生产链路成立。
性能分段统计解析排队、Host 元数据解析、事实提交、弹窗呈现、确认到执行，且将
用户等待时间单独排除；新鲜句柄复用和冷恢复复核分别测量，再据真实基线定预算。

### 已接线的请求关闭

状态：部分生产接线，2026-10-12。绑定 Run 终态、未绑定 Task 完成/失败/取消已接入独立关闭；
绑定 Task 的独立 Ensure 来源及首绑拒绝已有权限请求现已接线；跨端 UI 和撤销事件
接线仍未完成，不能宣称整个权限生命周期已经验收。

共享类型已落在 `nola_domain::permission_closure`：严格反序列化的 closure/source，
绑定来源内嵌 CorePermissionOccurrence（Run、Core request、effect），seal 使用
AssistantDigest，撤销事件游标使用 JournalNumber。`validate_for` 核对原请求身份与
时间顺序；它只做结构验证，不依据可选 Core ID 推断是否绑定，也不授予来源证明或
执行权限。当前 schema 72 的请求行包含独立 closure_json；绑定 Run 对账在同一
事务写 withdrawn/closure、撤销派生 grant、关闭 suspension、提交 Task 回执。
用户决定和时间原样保留。读取返回 closure，Authorize/Intervene 在 StopPending
之前返回 Closed；CLI 不再把历史允许解释为继续执行权。UI 专门显示尚未验收。

来源捕获已接入绑定 Task 的 Enqueue/Ensure/Suspend 事务：`permission_source::capture` 按当前
checkpoint revision/event cursor 的 delivery witness 查找唯一 pending permission
effect，复用 outbox 的摘要校验读取，再核对 Core ID、child、能力、输入和指导。
Store 生成 `CorePermissionSourceProof`，放入请求所有的 `permission_request_sources`，
与请求同事务落库。Suspend 的 `continuation_json.core_permission_source` 是返回给 Host
做回执完整性校验的副本，不是终态关闭的来源；终态只读取请求所有的证据表。
Host 提供该保留字段会被拒绝；只保存 occurrence、Host request ID、摘要和提交位置，
不复制工具输入全文。schema 72 新增不可变来源表，effect/Run 唯一，禁止跨请求复用。
不自动回填旧挂起记录。跨端关闭消费仍未完成；绑定终态已消费这份来源，缺来源拒绝
并回滚，不回填伪证据。
CLI 热路径回执校验允许且仅允许 Store 增补该类型化字段，其余原始 JSON 内容必须
相等；证据中的 Run/Core request 还须匹配原意图，任意附加字段不能被忽略。继续
执行时比较的是 Store 保存的完整 suspension，不剥离持久证据。

公开的 `DenyPendingForTask` 命令及 CLI/Application/Store port/Store 的批量拒绝
入口已删除，无旧格式兼容。CLI stop 不再在取消事务之外另写用户拒绝；真实用户
拒绝仍走 `Authorize(request_id, Deny)` 逐请求仲裁。两个自动合成 deny 写点均已移除。
绑定 Run 使用 seal；未绑定终态使用同事务刚提交的精确 Task 终态回执，关闭其名下
权限请求，不冒充用户决定或 Core effect 撤销。无正式终态回执时不能靠批量拒绝清理。

实施进展：Store resolve 已在决定/grant 写事务内检查 Task 取消意图，Store port 与
Application 新增 `StopPending { request_id, task_id, requested_at_ms }`；CLI 拒绝将
它消费成已授权或成功重放。取消时间缺失报不变量错误，不补时间。该门禁同样阻止
已停止任务的历史决定被当作新的许可回执；历史决定仍可 Get 读取。独立 closure
已接入绑定终态，跨端专门 UI 展示仍待验收。已批准的请求派生 grant 现于绑定
Task 的取消意图提交和原 Run seal 对账事务中撤销，保留决定与历史。当前来源关系
仍使用既有 request→确定性 grant ID、Task Host scope 的精确关联；不是用户文本
关键词分类。通用 grant upsert 现拒绝 `permission-request:` 保留 ID 命名空间，
请求决定事务使用首次 INSERT（不是 upsert），重放不再清空撤销时间。这样普通
配置编辑不能预占或复活请求 grant；类型化独立 provenance 和完整 closure 生命周期
尚未全部接线，不能把保留 ID 防护冒充完整来源模型。

## 已确认反例

修复前，`runtime_task_run_binding.rs::reconcile_runtime_task_from_run` 在原 Run seal 对账事务中
将所有 pending permission 改为 resolved/deny。`PermissionRequestStatus` 只有
Pending/Resolved，`PermissionRequestResolution` 只有 Authorization/Intervention。
因此执行结束、用户拒绝、用户返回输入框的原因无法完整区分。当前绑定终态已以
closure 替换该写点；未绑定所有正式终态及绑定 Ensure 来源也已接线。

修复前，`permission_transport.rs::resolve` 的会话范围允许与 grant 同事务提交，但未检查
Task 的取消意图。取消已提交后旧 UI 点击不得产生新的授权或执行权；执行 resume
已有 cancel_requested=0 检查，不等于权限决定与 grant 的生命周期已正确。

## 所有权与类型

用户拥有 Authorization/Intervention 决定；Core 拥有原 permission effect 的生命周期；
Application 验证精确身份并准备关闭；Store 原子保存决定、关闭证据、grant 与投影。
Host/UI 只能发送意图和读取投影，禁止替用户选择 deny。

请求保留 request_id、Task/child/Run/Core permission ID、能力、具体资源与版本。
增加独立 `closure: Option<PermissionRequestClosure>`，不把关闭原因塞进 decision。
closure 必填闭合 ID、请求身份、关闭时间和类型化来源；Core permission ID 放在
绑定来源的 occurrence 中；Task 级来源不携带或伪造 Core effect 证明：

- `run_sealed`：精确 Run/assistant ID、seal digest、原 permission effect 引用。
- `effect_withdrawn`：Core 已提交撤销事件及其游标，不以 Host 断线证明。
- `unbound_task_terminal`：无 Run binding 的 Task 已提交正式完成、失败或取消回执；证明该 Task
  的权限入口关闭，不证明某个 Core effect 被撤销。请求上的可选 Core ID 本身既不
  建立也不解除 Run binding；禁止用字段存在与否替代数据库中的实际绑定关系。
  Store 在回执写入的同一事务核对完整回执、Task 状态与结束时间及无绑定关系。
  关闭失败则 Task/attempt 终态、回执和 grant 撤销一起回滚。原终态原因留在回执中，
  不把失败写成取消或用户拒绝。schema 71 不保留旧取消专用来源别名，不自动迁移数据。

统一投影 status 为 pending/resolved/withdrawn：无决定且无 closure 为 pending；
有决定且无 closure 为 resolved；有 closure 为 withdrawn。withdrawn 可以保留此前
真实用户决定及时间，但它永远不是可恢复执行权。禁止把 closure 的时间填入用户
resolved_at，禁止用一个字符串同时表达请求关闭和用户拒绝。

Authorize/Intervene 返回当前闭域结果：首次提交、同决定重放、决定冲突、
`closed(closure, prior_decision)`、`stop_pending(task_cancel_identity)`、missing。
closed 和 stop_pending 不是拒绝授权，不触发模型重试；前者移除操作入口，后者
显示取消待确认并禁用按钮。同 ID 同载荷的原决定重放可返回历史事实，但必须同时
携带当前 closure，消费者不得据历史允许重新执行。

## 正常、竞争、取消与恢复

```text
pending ──用户决定事务──► resolved
   │                        │
   └──正式关闭证据事务───────┴──► withdrawn（保留已有决定）
   │
   └──Task 取消意图：不改用户决定；拒绝新的决定/grant，等待 Core 收尾
```

1. 决定提交在写事务内同时读取请求、精确 Task/Run binding、cancel intent、closure；
   不能用 Application 先读后写的快照替代这个仲裁。允许范围由原请求导出。
2. 决定先赢：保存原决定；随后取消不得改写它。取消意图或最终关闭撤销本请求派生 grant 的
   后续可用性，保留授权历史；不撤销其他请求或独立用户设置的 grant。
3. 取消意图先赢：Authorize/Intervene 返回 stop_pending，不生成 grant、不追加指导
   输入、不交付旧 permission continuation。Core 关闭后返回 closed。
4. 关闭与 Task seal 对账在同一事务中更新 closure、suspension、相关派生 grant
   可用性、Task projection、terminal receipt 与通知。任何一步失败全部回滚。
5. 重放原关闭 ID/相同来源返回原结果；不同载荷冲突。读取失败是 outcome unknown，
   不补 deny、不自动批准、不凭 UI 重试创建新的 request ID。
6. 冷启动从同一持久投影恢复。已关闭请求不能出现在 PendingForTask/PendingForOwner，
   但 Get/历史审计必须可读。孤立/缺失来源明确报证据问题，不能猜测已经关闭。
7. 未绑定任务运行中收到取消仍只有意图：撤销请求派生 grant，不提前关闭请求；
   WorkerFinish 或其他合法终态 writer 提交 cancelled 回执时，在同事务核验回执
   ID/完整内容、Task cancelled 状态、结束时间和无 Run binding，再关闭请求。
   尚未执行的队列任务可由取消事务直接提交该回执。已有用户决定一律不改写。

## 完整替换范围

单一当前版本切换，不新增旧格式别名或长期并行真相，不删除用户数据库。

### 存储与来源规格（部分实施）

来源证据必须在请求进入持久等待时绑定，不能到关闭时才从 delivery queue 猜测。

已有生产前置门禁：`suspend_runtime_task_for_permission` 在原写事务中查询 Task 的
canonical Run binding；存在 binding 时，continuation 的 Run、child、input watermark
必须逐项相等。冲突发生在权限请求 INSERT 和租约撤销之前，事务回滚，不产生新
请求、suspension 或事件。现进一步在同一事务捕获实际 Core effect 来源（见上文）；
无 binding 的普通进程任务仍走其自身协议。

`connection.rs::prune_settled_outbox_rows` 会删除 Core 已消费的 acknowledged effect，
且 `effect_outbox_payloads` 随外键级联删除；这是队列清理，不是权限历史作废。
因此不能用“终态时还能查到 outbox 行”作为 closure 必要条件，也不能为了闭合
权限无限保留整个请求 payload。来源契约如下，绑定 Enqueue/Ensure/Suspend 已实现捕获，
关闭读取请求所有的来源；不能以 Host 的 continuation 自报内容作为证明：

- 在 Ensure/Suspend 原事务中，以原 Run、Core request ID 查询实际待处理 permission
  effect，并核对能力、输入、child 和 Task binding；保存最小 occurrence、原 effect
  request ID、payload digest 及来源提交位置。不能接收未经核验的 Host 自报 effect。
- 来源绑定与 permission request/suspension 同事务提交；唯一关联禁止同一个 occurrence
  被复制成多个用户授权入口。随后 outbox 清理不能级联删除该证明。
- Task 首次绑定 Run 不得已有权限请求（包括已决定或已关闭的历史）；原请求不是
  新 Run 的权限来源。预检和实际绑定事务使用同一约束，拒绝结果为 Conflict。
  不迁移来源、不删除请求、不自动重试模型，也不回填 Core ID 来伪造来源。
  外层 admission UoW 回滚新 Run 与 effect；原 Task/lease/请求保持原样。
- seal 对账消费这份不可变绑定，再核对原 Run seal；缺绑定是证据缺失，不批量补造。
  Core 撤销路径另需核对已提交撤销事件和游标，不能用收到了 Host 响应替代已提交事件。
- 冷启动正例必须先产生真实权限请求，再关闭、清理 outbox、重开数据库，验证 closure
  仍可核验和重放。来源保留与完整 closure 的验收应分别记录，不能混称已经关闭。

当前 schema 72 的 `permission_requests.status` CHECK 接受 pending/resolved/withdrawn，
统一列集合和 reader 为 14 列，包含 closure_json。数据库 CHECK 约束决定与关闭形状，
触发器禁止已关闭请求被修改。新库按唯一当前 schema 原子初始化；不匹配旧库由
schema gate 无写入拒绝，不自动 ALTER、清空、迁移或补来源。本批未修改用户数据库。

采用请求行上的 nullable `closure_json`，它是业务上“尚未关闭”的可空字段，不是
兼容旧字段的默认。持久 closure 必须是严格类型化 JSON 对象，而不是任意 metadata：

| 字段 | 验证责任 |
|---|---|
| closure_id / request_id | 非空；请求归属相等；同 ID 异载荷拒绝 |
| task_id / child_session_id；绑定 source.permission 中的 core_permission_request_id | 与原请求及已提交 binding/effect 精确一致；未绑定来源不宣称 Core effect 状态 |
| closed_at_ms | 非负且不早于请求创建；已决定时不早于其决定；重放保持原值 |
| source | 闭域 tagged enum；其原 Run/事件/取消回执必须由正式仓储在同一事务验证 |

status 与 closure 的冗余只作受约束读投影：withdrawn 当且仅当 closure 非空。
pending 不得带 decision/resolved_at；resolved 必须带完整决定与时间；withdrawn
允许“无原决定”或“完整原决定”，禁止半份决定。数据库 CHECK 保证基本形状，
Application/Domain 校验类型和身份，来源验证仍属于正式事务，JSON 自称 seal 不算证据。
closure 一经写入不可覆盖；已关闭行的决定、时间和身份同样不可变。合法同载荷
重放必须在 UPDATE 前识别，不能依赖触发器报错后猜测成功。

读回/写入的必要顺序：

1. Enqueue/Ensure 只接受无 closure 的新 pending 候选。Ensure 找到原 Core request
   时返回其当前状态（可能 withdrawn），不能新建 ID 绕过关闭；相同身份载荷冲突
   继续拒绝，不复活旧卡片。
   Enqueue/Ensure 新建入口现均在原写事务检查 Task 存在、未请求取消且为可活动
   状态；停止意图与终态之后的迟到创建拒绝。Ensure 的原身份重放先于新建准入
   检查，已关闭记录仍可重放读取。绑定请求的新建还必须原子保留 Core 来源证明。
2. Authorize/Intervene 在一个写事务先读精确请求；若 withdrawn 返回 Closed，
   其次检查 Task stop intent，再处理同决定重放/冲突和首次决定。Closed 优先于
   StopPending，否则封存后的请求会永远被错误显示成“还在取消”。
3. Run seal 对账先验证正式 seal/manifest，再为该 Task 关联的请求创建 closure。
   关闭尚未决定的请求不能写 resolved_at/deny；已有决定保持逐字节不变。没有精确
   effect 归属的请求不能靠 task_id 批量伪造来源，必须拒绝并保留对账缺口。
4. 同事务撤销派生 grant、关闭 suspension、提交 Task receipt 与通知；closure 任一
   校验或写入失败则整笔回滚。禁止先对外通知“已关闭”再补数据库。
5. Get 导出完整 closure；PendingForTask/Owner 仅返回无 closure 的 pending。
   所有解析路径共用同一列集合与校验，不额外维护 Host 的已关闭 ID 集合。

CLI 热路径 `production_permission_wait_requires_exact_store_grant` 已改为实际领取
model effect、提交模型工具调用结果，由 Core 产生 RequestPermission，再交给生产
Task 挂起流程；核对持久请求与 continuation 保留相同的 Core ID 和输入，未授权前
没有 ExecuteTool。模型响应仍是确定性 fixture，不是在线模型验收。
冷恢复、取消及等待权限时预算停止场景现同样从实际 Core 工具调用产生权限请求。
它们验证来源匹配；取消用例现还核对独立 closure、原决定保留及重开重放。跨端
无挂起 Ensure→取消终态链路已有实际 Core fixture 验收：批准/未批准两分支都
从运行中 Task 保存请求来源，停止意图不改用户决定；closure 写入故障使 Task
回执提交回滚，解除故障从原 Run seal 恢复。整个过程无 suspension，队列清理后
来源及 closure 仍可重开/重放，没有第二个 Run/attempt。此用例使用确定性 Host
响应，不是线上模型或 UI 验收；正常完成/失败的无挂起分支仍需补充。跨端显示
和 scope_rule 权威来源也未完成，不能为保留测试通过放宽来源校验。

| 模块 | 同批必须修改 |
|---|---|
| Domain / Store port | closure 来源、身份校验、严格 outcome；关闭不是授权决定 |
| Store schema / transport | closure 持久字段与不可变约束；事务内决定竞争、grant 禁用与精确关闭 |
| Task/Run reconciliation | 删除终态对账里的 deny 写入，提交带 seal 来源的关闭 |
| Application | typed closed/stop_pending 读写及当前来源校验，不转为 Authorization::Deny |
| CLI Host / supervisor | 区分用户拒绝、关闭、停止待确认；不恢复旧 continuation |
| Mac/iOS/Android/Web | 删除或禁用卡片来自同一 projection；不从连接或 Task 字符串猜关闭 |
| 导出/测试/日志 | 保留决定与关闭两条事实，日志不输出工具参数、凭据或私有指导 |

## 必须取得的证据

要求真实 Application/Core/Store 正向链，而非只造一条 closure JSON。分别测决定先赢、
取消先赢、关闭先赢、真实并发仲裁、同 ID 重放/冲突、事务中断、ACK 丢失和冷重开。
检查原决定不变、无多余 grant/模型派发、精确 Run 及 effect、其他任务权限不受影响。
必须增加 UI 切会话/断线/冷启动场景：旧卡片无法批准，closed 与用户 deny 显示不同。
测量决定/关闭事务延迟、投影发布至 UI 移除延迟；记录失败和缺样本，再据实测定预算。
当前完成证据不足；现有取消测试只证明原 Run 封存与零新增派发，不证明权限关闭语义。
# 拒绝后的任务结果契约（2026-10-13）

用户拒绝会关闭当前授权请求；它不是仍在等待授权，也不是模型协议失败。
Core 的已记录拒绝决定禁止本轮工具重试与替代路径。允许一次无工具的说明回答，
但回答存在不将外部任务提升为 Answered 或 ActionCompleted：在没有优先的取消、
执行失败或未知副作用时，收尾写入 permission.denied 阻断事实，并封存 Blocked。
恢复条件是新的用户请求授权操作或改变范围，不能由模型自行解除拒绝。
这条结果由 Core 决定，不要求拒绝后的模型调用 requirements_update（此时工具已关闭）。
拒绝验收明确期待 blocked，并验证目标文件不存在；批准写入验收明确期待
action_completed 且文件内容正确。两者都应有完整结果记录，不能共用“有回答即成功”。
