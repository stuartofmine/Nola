> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 模型配置重构：来源、目录、覆盖与冻结调用

状态：2026-10-19 源码审查后的目标设计，尚未整体实现。不得把本设计当作已完成验收。

## 当前实现与目标类型的对应（2026-11-08）

本文的模块化模型是职责目标，不是已经全部替换完毕的代码结构。当前实际类型/入口包括 `ModelConnection`、`ModelProfile`、`ResolvedRemoteModelProfile`/`ResolvedModelRuntime`、Host route/generation binding，以及 provider `PreparedModelRequest`。profile source/parameters 仍有 JSON 存储/边界，不能将表中逻辑 `ResolvedModelInvocation` 当作全产品已经共用的同名 DTO。

连接导入事务、会话 sparse overrides、能力声明/精确 reasoning selection、Host 私有凭据与已编译请求各有时点。目录启用、选择默认、配置探测、运行准入和物理网络请求是不同动作。当前实现映射与真实差距见[架构源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)，精确参数规则见[生成参数合同](GENERATION_OVERRIDE_CONTRACT.md)。

## 事故与根因

1. Core新建远程profile写入topK/repetitionPenalty=null；Apple共享ModelParameters将其定义为非可选整数/浮点。Mac模型选择在解码阶段失败，用户看到回车不发送。
2. MacLocalAgentRuntime以topK!=40和repetitionPenalty!=1判断是否发送；参数是否显式设置被错误替换成与UI默认值的数值比较。
3. ConversationParameterSnapshot读取把null重新补成40/1，保存整对象；一次无关编辑也可能把本地采样参数带进Responses。
4. CLI文件是显式导入入口，Core目录是运行权威，但启动、apply-config与模型中心的关系未在产品入口解释；缺少默认引用和凭据时只抛底层错误。
5. 未知模型能力曾被固化为false；改成工具默认true只能修正产品默认值，不能替代能力来源、显式覆盖和provider声明的完整设计。
6. 模型目录、UI显示参数、请求参数混在parametersJSON。Rust/Swift/Kotlin/TS各自解码、补默认和过滤，缺少同一份跨语言契约样本。

## Magpie源码依据

本地参考：参考资料/magpie；审查版本dd70c5e6ae58cc3623571e62661718c6ce8a3a51。未运行Magpie、未读取第三方真实账户、未执行其安装脚本。

| 源文件 | 核对到的实现 | Nola采用与边界 |
| --- | --- | --- |
| internal/provider/provider.go | Provider分别持有各协议base、账户、模型选择；配置读取失败与空目录区分 | 连接、协议路由与模型选择独立；不要一个baseURL推导所有协议；保留Nola私有凭据库而非照搬明文key |
| internal/provider/importapps.go | AppImport标识new/same/taken、KeyOf、Skip；提交时重新检查来源与冲突；同地址不同key可归入账户 | 导入分预览和提交，给出逐条结果；不得覆盖订阅账户或向不同origin转移凭据 |
| internal/provider/import_opencode.go | API key和OAuth/wellknown分流；无可信端点的key不导入；鉴权数据读取有显式入口 | 来源适配器输出类型化候选，不把订阅token当API key |
| internal/provider/import_codex.go | 按provider收集顶层及profile模型；按wire_api选择协议，保留header信息 | 不只导入当前一个模型；配置来源、账户来源、模型目录分别处理 |
| internal/provider/codex_store.go | 按来源指定的存储位置读取登录；缺失、不可读、ephemeral区别；刷新回写原存储 | 借鉴来源身份与错误分类；Nola不因参考代码而获得回写外部登录授权 |
| internal/provider/detect.go | 模型列表GET与最小推理探测区分；context取消；有界等待；支持草稿模型探测 | 发现不是能力验证；用户主动测试才发推理请求；普通编辑或启动不自动计费探测 |
| internal/catalog/live.go | live目录与选择分离，保留抓取时间、base和协议侧；窗口/输出/模态/effort为独立元数据 | 刷新只更新发现快照，不覆盖显式模型参数与会话 |
| internal/catalog/fresh.go | 后台单飞刷新、TTL及超时；CLI和测试不隐式联网 | 缓存可刷新但不阻塞发送；失效缓存不能覆盖有效配置 |

不照搬：历史ID别名、迁移恢复逻辑、订阅私有接口、按模型名猜测、默认无限路由重试。Magpie是网关；Nola还有会话与任务执行所有权，不能把全局路由切换直接套到运行中的会话。

## 对象与所有权

| 对象 | 所有者 | 输入 | 输出与禁止事项 |
| --- | --- | --- | --- |
| Connection | Application | 名称、显式协议路由、账户引用 | 稳定ID/revision；不含模型生成参数 |
| Account / CredentialReference | Host凭据服务 | API key或来源授权 | 可用性与私有引用；公开DTO不含secret |
| CatalogSnapshot | Provider adapter + Application | 连接revision、GET响应 | 模型原始ID、限制、能力、来源时间；不决定执行权限 |
| ModelDefinition | Application | connection ID + protocol + model ID，用户显式覆盖 | 选择身份；同名模型不合并；不混入UI偏好 |
| GenerationOverrides | 会话/模型各自的配置聚合 | 可选参数、清除/继承指令 | 保留unset与显式值；不填本地默认值 |
| LocalInferenceSettings | 本地运行时 | 本地模型参数 | 必要的确定数值；不得作为远程DTO |
| PresentationPreferences | 客户端会话设置 | 折叠、流式展示、样式 | 不能进入provider采样参数 |
| ResolvedModelInvocation | Core解析器 | profile revision、会话覆盖、调用场景 | 冻结route、有效参数、能力来源、credential ref；Host只取密钥和发送 |

实现优先复用现有连接/账户/目录命令，不另建平行数据库。将sourceJSON/parametersJSON边界替换成类型化解码和错误路径；内部JSON存储不等于允许所有调用者自行解释字段。

## 参数规则

- 选择身份与参数一起提交，CAS拒绝冲突；读取配置不能触发写回。
- 优先级：会话显式值 > 模型显式值 > provider声明默认值；无声明则让服务端使用默认，不自行发0.7/0.9/40/1。
- provider窗口/输出上限是限制，不自动成为每次请求的max_tokens。
- null表示当前契约中的未设置；覆盖命令区分inherit、clear、value，不能以数值等于默认值判断。
- 能力优先级：用户显式覆盖 > provider明确声明 > 产品基线。未知toolCalls=true；其他未知不宣称已支持。权限由执行策略另行决定。
- 协议切换只接受目标协议支持的参数；明确不支持的显式参数返回字段级错误，不能静默删除用户值。
- 目录刷新不改正在执行的冻结请求；会话恢复读取自身选择，不重新套用全局默认。
- 元数据必须来自具体endpoint/model/protocol，不能用同名模型的别处能力冒充证据。

## 配置与调用流程

来源适配 → 类型化候选 → 预览校验 → 凭据保存/连接CAS提交 → 可取消目录发现 → 用户选择模型 → 会话配置事务 → Core解析冻结 → Host取凭据 → 协议adapter编码。

文件导入与模型中心是同一命令端口的两个编辑入口。一次应用返回每个连接的成功/失败及默认选择结果；不得在失败时伪报全部成功。已有凭据引用不应仅因当前shell缺少环境变量而失效；换密钥是显式动作。自动读取外部账户、上传信息和推理探测不是文件读取的附带动作。

### 已实施的导入事务边界

Host 将凭据保存到新的私有引用后，只提交一次 `ModelCommand::ImportConnection`。Application 将账户 CAS、连接 CAS、目录校验和模型选择放在同一数据库事务；直接调用和服务命令调用都适用。任一步失败，数据库全体回滚，旧账户引用和模型选择保持不变。

外部凭据库与 SQLite 不是分布式原子事务。失败可能留下未引用的私有条目，不能谎称全部回滚；尤其提交结果未知时，不得直接删除可能已被引用的凭据。未引用条目不会被运行路由使用，后续回收需要独立的可达性核对。导入不得轮换被其他连接共享的账户，当前明确拒绝并要求单独绑定。模型读取不为此开启写事务。

## 错误与交互契约

ConfigurationError包含stage、connectionID、modelID、fieldPath、code、recoverable及公开说明；不含secret。解码错误必须能指出parameters.topK，禁止只显示“数据丢失”。

发送入口在prepared失败时保留草稿，展示明确阻止原因。Enter与发送按钮调用同一提交动作；IME确认候选不发送；不能把配置失败吞成无响应。

## 验收与性能

- 同一JSON fixture经Rust→Swift/Kotlin/TS解码及保存后，unset、0、false、空停止词与显式值含义一致。
- 本地MLX和远程Responses/Chat/Anthropic分别验收，不以一个万能参数对象覆盖。
- 两连接同名模型、无密钥服务、多账户、失效凭据、目录无元数据、显式禁工具均覆盖。
- 覆盖启动→选择→发送→切换→重启恢复，以及目录刷新与会话编辑并发。
- 冷启动不执行模型探测；热发送不获取模型目录、不枚举Keychain。目录读取异步且可取消，首屏读持久快照。
- 记录配置解析耗时、目录请求数、凭据读取数；性能数值在实测前不得称已达标。
- 测试仅隔离数据；不自动转换现有用户库。全平台构建和真实交互验收通过前，不宣称完成重写。

## 当前接口归属（2026-10-28核对）

当前账户、连接、catalog、profile与probe用例集中在Application ModelCommand；会话选择通过SelectConversationExecution配置CAS提交。paired execution-selection只修改源会话，不能修改Host默认模型。服务凭据与公开profile分离；恢复按冻结profile/config身份解析，不能用最新界面选中模型替换原Run。字段入口见[当前接口索引](../01-领域与协议/INTERFACE_CATALOG.md)。

## 选择模型不是散写多个设置字段（2026-10-28源码核对）

`SelectConversationExecutionInput.into_patch`将一个选择意图变成一次严格配置CAS；输入包括conversation_id、expected_revision、selection、可选执行位置/backend/session、additional_configuration与updated_at_ms。

| selection.kind | 同一次patch的变化 | 目的 |
| --- | --- | --- |
| inherit | 清空modelSelectionID、executionHostBinding、clientHostParameters | 恢复继承，不把上一次Host参数残留当成当前模型参数 |
| location | 设置executionHostRawValue | 仅切换执行位置，与模型推理位置分开 |
| model | 设置modelSelectionID；清空Host binding/参数；modelOverrides与presentation按该model_id写入 | 模型选择与其参数/展示共享一个CAS |
| host | 清空modelSelectionID；设置binding、Host参数和execution-host展示 | 不能同时持有普通模型与Host选择两套有效来源 |

顶层execution_host与location重复提供时拒绝；additional_configuration不能覆盖selection拥有的字段，也不能覆盖本patch已写字段。因此先改模型、再独立写绑定/参数的客户端实现不满足原子意图契约。普通配置变更可与选择一起提交，但不能绕过该重叠校验。

字段缺省与显式null不同：例如seed只接受带kind的意图（clear或value），value是规范整数文本，裸数值与非规范数字字符串拒绝。Host binding不能带apiKey，baseURL不能藏user/password、query token或fragment；展示参数不是凭据传输通道。

并发情况：A、B读取同一configuration revision后各自修改，只有满足CAS的一方提交；另一方保留编辑并重读，不能把本地最后一次选择覆盖源结果。源selection已提交后首次发送仍须读取submission frontier；已绑定Run恢复则保留原冻结配置，不把最新选择反向应用到旧Run。

当前错误边界部分以String返回，例如 `additional configuration overlaps execution selection`、`execution location must have one source`；HTTP包装层也可能返回业务result而非错误状态码。客户端应按现有结构化结果及持久回执判断，不将错误文本匹配设计成业务路由或授权依据。文档不把这些String接口宣称为已完成的全类型化错误体系。

## 2026-11-03：模型接入入口与订阅目录续期收口

本次继续使用原有 Connection / Account / Catalog / Profile 权威链，不另建模型管理存储。

- Mac 首屏明确分成 API 连接、ChatGPT 授权、来源文件导入三个入口。文件导入仅处理来源 API 配置，不将 Codex 登录或其他订阅 token 当 API key。预览同时显示密钥是否包含，订阅候选可转入独立 ChatGPT 授权。
- 授权保存不等于模型可用。用户在同一账户页主动获取目录后直接进入该连接的模型启用页；连接详情也可直接管理所属 ChatGPT 账户及刷新目录，不要求用户往返另一个页面寻找操作。
- 已确认缺陷：推理 dispatch 已会更新近过期 access token，但目录读取只拒绝过期 token，导致已登录账户闲置后不能发现模型。目录现在在相同账户锁内复用现有刷新实现；原生提交绑定刷新后的精确私有 session revision，且保留 client/subject 身份检查。未知刷新结果仍拒绝重复使用旧 refresh token，不换账号或退回 API key。
- 当前 OpenAI OSS SIWC 文档确实规定 `dynamic_agent_client` 注册、公开 `/v1/models` 的 `models/visibility/slug` 和 `/v1/responses`；这与 Nola 现有独立注册路线一致。此前对其是否为无效私有协议的疑问经官方文档核实消除。Magpie 的 API 配置导入与来源订阅登录也分别实现，不能因用户称“GPT 账户导入”就混成一种密钥。

参考： [Magpie API 配置导入](https://github.com/yetone/magpie/blob/main/internal/provider/import_codex.go)、[来源登录存储](https://github.com/yetone/magpie/blob/main/internal/provider/codex_store.go)、[OpenAI 注册与登录](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)、[目录与推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)、[会话续期](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)。

验证边界：合成 issuer / 目录 fixture 验证目录续期、身份与请求数量，不等于用户真实 ChatGPT 账户已登录可用。真实浏览器登录、账户资格及服务返回仍需要正常用户授权流程；本次不读取来源 token，不代替用户登录，不写入日常配置。

补充目录能力接线：当前官方 Codex `ModelInfo` 的 `supported_reasoning_levels[].effort` 是真实字段，而不是按模型名推断；参见 [openai_models.rs](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/openai_models.rs)。SIWC 目录只在响应实际包含该字段时保留它。共享 Application 在用户显式启用模型时把这些 level 解析成既有 `reasoning.control=effort`，新建和重新启用 profile 共用此路径；已有显式 reasoning 与用户能力覆盖仍优先。目录更新不隐式覆盖 profile，Mac 允许再次应用同一启用集合以明确采用新目录能力，不要求先取消勾选。缺字段保持未知，不照搬 Codex 打包目录冒充当前账户证据。订阅模型也开放原有显式能力设置，禁止 API-key 探测路由。

发行资格边界：仓库当前为私有，源文件上的 Apache-2.0 标识本身不证明满足 OpenAI OSS plan-sharing 的分发资格。公开发行/商业托管条件仍待确认；此处只修公开协议接线与本地交互，不能据此声称真实订阅获准使用，也不会代用户完成登录或创建持久授权。

`reasoning_levels` 只证明 effort 选择，不证明公开摘要。官方目录另有 `supports_reasoning_summary_parameter`；显式 false 必须被保留，不能把“支持思考”同时解释成“支持 summary:auto”。本轮以可选 `CatalogModel.summary_supported` → profile `reasoningSummarySupported` → runtime `reasoning_summary_supported` 独立保留至冻结请求。显式 false 禁止请求摘要，true 允许按用户设置请求；SIWC 未知不自动请求摘要，普通 API 未知继续遵循既有显式设置。尚未取得真实服务摘要前，不得标记订阅思考展示已验收。

### 授权失败回传闭环（同轮补验）

只读追踪已覆盖：Mac 显式同意 → 同 owner 原生桥 → 随机 host/state/nonce/PKCE 与 loopback callback → discovery 的同 issuer 端点 → resource/scope/code 交换及 ID-token 校验 → 私有会话保存 → Core 账户 CAS → 所属账户目录 → 用户启用 profile → 冻结 route/account revision → 专用 Responses 传输。没有复用第三方登录 token，普通 API key 与 SIWC 不互相 fallback。此调用链存在不等于真实账户资格已通过。

此前实际错误断点在原生 flow 投影：worker 抹平所有失败，Mac 仅观察 `failed` 阶段而忽略 `error`。现在只保留固定的拒绝、过期、网络不可达三类公开错误；不透传供应商响应、callback URL 或 error_description。Mac 在失败阶段显示相应原因与下一步。新账户明确拒绝/过期不会再误称可能保留授权；已有账户保留原私有会话的可能性仍如实提示。

定向补验只走生产 NativeSiwcService 和隔离 issuer 的 access_denied callback，检查终态失败、无账户保存、无 token exchange/目录/推理请求及错误描述不外泄。没有真实注册、浏览器登录或修改日常配置。其他服务端资格/授权失败仍需正常用户流程中的真实返回才能确定，不能以该 mock 证明其已获准。
