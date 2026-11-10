# 连接、账户与来源导入

2026-10-19 实施设计与支持矩阵。下文明确区分当前代码与未实现目标；测试证据由当前任务总账单独记录，不以设计文字代替验收。

## 所有权

沿用 `ApplicationCommand::Model`、`ModelRepository` 和现有 `model_profiles`，不建立第二套调用配置。连接及账户各自以独立 revision 保存到现有 `product_settings`，键空间由模型仓库独占；不迁移旧 profile、不自动读取其他应用凭据。连接保存可没有模型；目录发现失败不阻止保存连接。

连接 ID 与账户 ID 是用户配置身份，URL、显示名和协议不参与身份生成。账户记录只包含来源、标签、凭据句柄和登录状态，秘密由 Host 凭据库持有。Kimi Code、Kimi Desktop、Kimi Work 是不同来源；未实现的登录/刷新操作必须报告不支持，不能从订阅名称推断 API 权限。

## 命令与执行

- `save_account` / `get_account` / `list_accounts`：账户独立 CAS 写入；记录来源与安全句柄。
- `save_connection` / `get_connection` / `list_connections`：连接独立 CAS，保存显式协议集合和账户引用，不触网。
- `refresh_connection_catalog`：替换服务商目录快照，记录来源及观察时间；不改变用户启用/默认选择、不删除已有运行 profile。目录项由协议和模型 ID 联合标识。
- `configure_connection_models`：用户显式选择目录模型，产生或更新现有运行 profile；手动模型通过目录输入加入；来源未声明能力时，运行配置采用工具调用默认开启的导入基线，其他能力不自动开启，来源证据仍保留 unknown。运行 profile 引用连接及账户；冻结调用继续使用既有请求流程。

每次修改在仓库事务内比较连接 revision 及涉及的 profile revisions，再原子提交连接快照和实际运行 profiles。修改 URL、协议集合或账户会使旧能力证据失效。连接配置与目录、用户选择分别建模，不用目录刷新隐式恢复用户禁用项。

## Host 导入与探测

Host 只在用户指定来源后读取。发现结果使用不含秘密的候选计划；确认前重新读取来源并比较版本/摘要。凭据仅写 Host 凭据库，Core 接收句柄。按候选逐项提交以上命令，保留部分成功；重复来源、名称冲突、地址变化与自身网关回环由导入计划明确报告。不得把 UI 提交的来源路径视为读取授权，不执行来源中的 shell 命令或环境模板。

协议列表来自来源适配契约或用户显式选择；不凭 provider 名称猜测。目录来源声明、主动文本探测证据及独立用户覆盖分开保存。Host/Web 探测先显示所选账户、模型、协议、最大请求数 1 及可能计费的明确确认；三并发、可取消，连接 revision、账户 revision 及实际凭据槽 revision 的 opaque binding 共同约束，旧结果不得覆盖新状态。工具、视觉、推理等未探测项保持 unknown，不从最小文本成功推断支持。

## 路由与 usage 边界

Nola 自有会话复用既有 `ModelAttemptRecord`、usage 部分归并与会话累计存储。账户选择在请求冻结时固定。首内容前重试也必须尊重是否已提交、是否可能有副作用；首内容后错误不跨账户/协议拼接，不重放工具副作用。外部客户端网关请求不属于任意 Nola 会话，不得向会话 usage 虚假累计；使用独立 `GatewayAttemptRecord` 持久账本，原样透传上游 usage。

## 尚需集成验收

来源读取、网络目录、外部网关与客户端入口已接线，其隔离测试与真实应用验收必须分别记录。Codex、Kimi Code、OpenCode公开来源管理均已有假进程/假HTTP验证，但这些来源登录不授予Nola订阅推理权限。独立Nola SIWC的授权、刷新、撤销及专用Responses传输另见 `NOLA_SIWC.md`，不能与导入第三方token混为一谈。没有运行真实OAuth或付费账户验收；纯Core事务与协议fixture通过不代表真实计划资格或供应商服务已验收。

## 2026-10-19 当前代码接线

`cli/src/model_connection_host.rs` 实现用户明确指定绝对路径的 JSON/JSONC/TOML 读取、私密预览计划、十分钟有效期及确认前来源重读。计划仅存在 Host 进程，UI 只接收随机 token 和脱敏候选；独立 CLI 进程不能复用另一进程的 token。来源配置只读，外部 auth 文件和来源显式环境变量必须分别由调用者授权。禁止执行 auth 命令、展开文件引用或猜测未知供应商密钥目的地。

| 来源 | 当前支持 | 明确不支持 |
|---|---|---|
| OpenCode | 显式 `npm` 对应 OpenAI Responses、Chat Completions、Anthropic；options.baseURL、models、API key / 已指定 auth.json 的 api 条目 | 未声明 SDK/URL 的内置供应商推测、OAuth、任意自定义 headers |
| Codex | 明确 provider 配置、官方 Responses 默认、选中模型、API key 或授权读取的 env_key | 已登录 ChatGPT token 直接复用、auth command、自定义 headers/query_params |
| Kimi Code | 显式 kimi/openai/openai_responses/anthropic 类型、providers/models、api_key/api_key_env | Gemini/Vertex 等未实现协议、任意自定义 headers |
| Kimi Desktop / Work | 独立来源身份与明确不支持错误 | 不声称可复用桌面登录或 Work 订阅 |

来源依据：[OpenCode providers](https://opencode.ai/docs/providers/)、[Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)、[Kimi Code 配置](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/configuration/config-files.md)。这些是来源适配，不是 Nola 旧格式兼容。

未知模型不再要求用户逐项勾选能力才能保存或启用。目录保留 unknown；运行 profile 使用文本协议最小能力基线，并标记 `executionCapabilityBasis=protocol_text_baseline`，不据此宣称工具、视觉、推理或采样支持。明确来源声明标记为 `catalog_declaration`。目录刷新不改变已有用户选择。

`discover_catalog_request` 仅对保存连接执行显式 GET `/models`，三并发、十秒超时、响应体两 MiB 上限、禁止重定向；401、403、429、5xx 和缺少目录分别报告。它不发 completion，也不能当作模型推理能力探测。

Host 目录取消使用 request UUID + 数据库 owner + connection/revision 绑定。Web `POST /api/model-connections/discover` 和 `/discover/cancel` 都要求 `{request_id, connection_id, expected_revision}`。取消与最终 Core CAS 提交同锁线性化；取消获胜时丢弃异步 HTTP future，不等待十秒超时、不保存晚结果；提交已完成时返回 `cancelled:false`，不宣称撤销既成写入。取消先于开始的短期墓碑防止请求重排，句柄最多64个、60秒有效，页面关闭会尝试取消但不把浏览器断连当服务端确认。连接 revision 改变由既有 Core CAS 拒绝写回。取消仅终止本地目录请求，不承诺远端服务立即终止计算。

Apple 目录使用独立 ephemeral URLSession，禁用 cookie/共享 credential/cache，拒绝重定向，三并发预算、十秒请求与资源超时、读取过程中两 MiB 上限；只向已绑定账户的精确协议地址追加 /models，不猜测额外 /v1。URLSession Task 取消并在提交前检查 cancellation；界面提供 Cancel discovery，退出页面取消活动请求，保留原目录/选择。已进入同步提交的结果不可回滚；界面重新加载已保存状态，不宣称已提交目录被撤销。Apple/Host 使用各自平台传输实现，相同限制不意味着已共同通过全部平台验收。

本轮隔离验证：Rust `model_catalog_discovery` 1/1、`model_connection_host` 9/9（含慢响应主动取消与 revision 失效拒写）；Mac `MacModelConnectionServiceTests` 4/4（提前取消、实际/声明体积超限、重定向拒绝、匿名连接）；Web 类型检查 0 errors/0 warnings。测试不使用真实账户，Mac Test 配置结果为 `/tmp/nola-catalog-boundary-validation.xcresult`；iOS 同构传输的业务回归不借用 Mac 结果冒称已验收。

后续集中验证：`cargo test -p nola_cli --lib model_ --locked` 为226通过/0失败/1既有忽略，包含文本探测、native bridge与CLI确认/覆盖解析；Core `model_connections_e2e` 12/12。Mac最终Test配置共26/26，其中账户服务5/5、导入/探测私有管道4/4；`/tmp/nola-mac-final-room-models-r3.log`。管道fixture暴露了Foundation整段读取对持久双向pipe的互等，实际服务已改4096字节有界POSIX读取，原帧上限保持。上述结果不包括随后新加的Codex source-login假进程测试，也不包含真实登录/计费账户。

### 主线 B 的明确缺口

| 操作 | 当前状态 | 不能据此声称 |
|---|---|---|
| 只读来源导入、API key连接、目录/选择分离 | 已实现，测试/端到端证据见总账 | 支持任意来源认证 |
| Host目录主动取消、revision冲突不回写 | 已接线，隔离慢HTTP与CAS测试独立验收 | 取消远端已经完成的工作、撤回已提交结果 |
| 主动协议/能力探测和费用确认 | Host/Web 单请求文本探测已接线，待本轮隔离验收；无自动付费探测 | GET目录或文本成功证明工具、视觉、推理均可用 |
| 能力声明/探测证据/用户覆盖三层 | 独立声明/文本证据/显式覆盖及运行配置投影已接线，待验收 | 未探测能力被否定或自动授权 |
| Codex/Kimi本机登录状态引用 | 只读引用/状态重读已实现，推理权限仍false | 登录复用、刷新token或订阅推理 |
| Codex本地owner登录与token刷新 | 独立显式登录/刷新、轮询/取消，假进程3/3通过；仅本地Host | 来源状态重读会刷新token、订阅HTTP推理已启用 |
| Kimi Code本机服务登录/状态/取消 | 已接owner管理模块与独立页面，假HTTP 3/3；OAuth刷新归Kimi | 读取状态会刷新OAuth、Agent session可当模型推理网关 |
| OpenCode provider登录 | 公开provider方法/authorize/callback隔离fixture 2/2通过；无统一来源取消/刷新接口 | 本机forget等于远端撤销、callback完成等于Nola推理可用 |
| 独立Nola SIWC | 同owner授权/刷新/退出、目录/profile与专用传输已接；Rust及Mac隔离验收见专项设计 | 第三方token导入、真实订阅资格已验证、远端公共网关可共享授权 |

参考实现核对：Magpie [`detect.go`](https://github.com/yetone/magpie/blob/main/internal/provider/detect.go) 将目录获取与最小推理请求区分，并通过 context 取消/有界并发控制请求；Nola只实现上述显式确认单请求边界，不复制供应商名称猜模型与私有订阅推理路径。

API key 写入 Host 私有凭据库，每协议绑定精确 URL。主调用、保存会话恢复及子 Agent 的冻结凭据引用已区分继承凭据、端点凭据、账户凭据；账户不可用或路由变化明确失败，不借用其他账户。重复导入保留账户身份，以预览时账户 revision 作 CAS。跨 Core/Host 凭据库不存在分布式事务：导入中途失败可能保留新建账户/凭据或已保存连接，结果返回阶段性失败；不自动删除用户已提交的数据。

## 本机登录引用

### 公开登录生命周期核对（2026-10-19）

以下区分公开协议、实际实施与外部条件；来源账户管理成功不等于Nola模型推理获准：

| 来源/授权所有者 | 可公开核对的生命周期 | 最小实施切分与限制 |
|---|---|---|
| Codex 本机 app-server | 官方 `account/read`、`account/login/start`、`account/login/cancel`；managed 模式由 Codex 保存并刷新 token，`refreshToken:true` 可显式刷新 | 账户管理子进程适配假进程3/3通过；不创建thread/turn、不读取缓存token。必须明确选定可执行文件与Codex home。不能推广为远端公共网关认证。 |
| Nola 自有 Sign in with ChatGPT | 官方 OSS 动态注册、PKCE/OIDC、refresh/revocation 与 Responses 推理契约 | 独立client registration、host ID、验证后的identity与私有token槽；授权生命周期、目录/profile和专用传输已实现并有隔离测试，详见 `NOLA_SIWC.md`。真实用户资格与真实推理未验收，不能导入Codex token替代。 |
| Kimi Code 本机服务 | 当前官方认证 REST 提供 auth snapshot、device login 启动/轮询/取消；登录材料和刷新仍归 Kimi owner | 显式连接受认证本机服务的管理切片假HTTP 3/3通过；绑定server identity/flow，不扫描token文件。服务session会驱动Agent/tools，不能冒充模型HTTP网关。 |
| OpenCode | 官方本机server提供 `/provider/auth`、`/provider/{id}/oauth/authorize`、`/provider/{id}/oauth/callback`，分发provider/plugin认证方法 | 已接本机Host owner/provider/method/prompts/callback，隔离fixture通过；不是统一OAuth issuer。无统一flow取消/refresh或稳定server_id，不伪造这些能力；不复制插件token、不启用Nola推理。 |

Codex 依据：[官方 app-server Authentication](https://developers.openai.com/codex/app-server#authentication)。Nola 自有授权依据：[注册和登录](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)、[账户和刷新](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)、[预览限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)。该订阅 Responses 路由要求 `stream:true` / `store:false`，不接受 `max_output_tokens` 等字段，不能直接套用当前 API 单请求探测体；新账户必须完成用户授权、OIDC 验签和 granted scopes 检查后才可能启用。

Kimi 依据：[当前官方 server API](https://www.kimi.com/code/docs/en/kimi-code-cli/reference/server-api.html)、[当前源码 OAuth wire types](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/types.ts)。其本机 REST 标为实验接口，应核对连接服务的 authenticated OpenAPI schema，不以历史 Python CLI 格式作回退。OpenCode 参考：[官方 Server Provider API](https://docs.opencode.ai/docs/server/#provider)、[官方 CLI](https://opencode.ai/docs/cli/)、[Magpie 来源适配](https://github.com/yetone/magpie/blob/main/internal/provider/import_opencode.go)。OpenCode当前本机服务使用显式HTTP basic认证，`/doc`发布当前OpenAPI，后续不能把登录入口缺失当作不实现的证据。Magpie [Codex store](https://github.com/yetone/magpie/blob/main/internal/provider/codex_store.go) 会写回来源轮换 token；Nola 目前没有这一行为，也不因此获得覆盖来源登录的授权。

本次核对只访问公开文档和源码，没有读取用户凭据、启动来源程序、发登录/刷新或计费请求。后续实现先完成账户 owner/revision 绑定、显式登录确认、取消、刷新串行化、失效与账号变化拒绝，再接推理；不将 API key 导入或“重新读取状态”标记为登录生命周期完成。

本轮选定 Codex 本地 owner 垂直切片：Host 接收用户明确确认的 executable/CODEX_HOME、账户 ID/revision 与 request UUID；仅启动该可执行文件的 app-server stdio，私有环境不继承 API key。请求仅限 initialize、account/read、account/login/start、account/login/cancel，不调用 thread/turn/logout。登录与 token refresh 是不同显式操作；刷新只允许已绑定的相同 owner。待完成期间轮询不读取来源凭据；取消、超时、子进程结束、账户 CAS 冲突都不保存晚结果，不删除/回滚来源程序已经完成的登录。成功只保存 owner 引用并保持 inference_supported=false，不把来源登录当作订阅 HTTP 推理支持。同一 owner 的在途操作串行；UI 明确告知会修改所选 Codex home 的登录，来源程序拥有令牌与刷新。测试使用隔离假进程，不运行真实 Codex。

`model_source_login.rs` 与 Web 来源页已接上述流程，假进程3/3通过；HTTP `source-login/start|status|cancel` 统一使用下文来源管理同源JSON门禁，仅允许loopback来源，远端Web认证不解除此限制。进程等待180秒、单帧1MiB、注册表32项；公共返回不含可执行文件/home私有绑定或凭据句柄，授权URL仅当前待登录页面内存持有、响应no-store。官方 `account/read` 只公开email/planType，不提供可验证的subject/workspace身份；刷新前后email变化会拒绝，但相同email不证明同一workspace。因此这里的账户身份严格是选定来源owner，不用于推理授权。对强订阅身份有要求时必须采用独立OIDC授权，不能偷偷读token来弥补。

刷新复用原owner槽，不制造历史槽；新登录CAS失败仅清理本操作新建且未引用的私有owner槽，原引用及来源token不动。子进程不继承API key/代理凭据，仅保留原HOME/PATH/TMPDIR/LANG/SYSTEMROOT/USERPROFILE公共运行环境并单独设置明确选择的CODEX_HOME。环境缺失导致来源无法运行时明确失败，不扫描其他凭据补救。同home锁保持到进程退出回收，而不是仅等UI报告取消。

隔离验收：`cargo test -p nola_cli --lib model_source_login --locked` 3/3通过（`/tmp/nola-model-source-login-final-tests.log`）。假进程覆盖登录→来源认证→同owner显式刷新、刷新复用引用、取消/并发owner拒绝/错误账户handle/CAS拒晚写，以及确认与授权URL边界。没有启动真实Codex，没有读取真实账户或发送登录/刷新请求；不将fixture通过声明为真实订阅可调用。最终CLI binary构建已通过；Web状态生命周期由主代理独立验证。

### Kimi Code 本机服务管理切片

用户显式输入已启动Kimi本机服务的loopback origin及其server bearer；这个bearer是本机服务控制凭据，不是Kimi OAuth access/refresh token。Nola不扫描server.token、不启动Kimi、不使用dangerous-bypass-auth。连接先读取已认证 `/api/v1/meta` 的server_id、`/openapi.json` 的当前端点契约和 `/api/v1/auth`；确认的origin/server_id及server bearer仅由Host私有owner引用保存，Core仍是Unavailable，推理权限false。域名不解析为loopback：只接受明确127.0.0.1或[::1]及端口，禁止重定向、代理、任意附加path/query；HTTP请求有超时与流式体积上限。

登录启动、状态、取消都绑定Core账户revision与所选server_id。只调用managed:kimi-code的login接口，不创建session/turn，不请求usage/userinfo，不调用logout，不把server模型ready当成推理授权。OAuth token自动刷新仍归Kimi；Nola状态重读不声称刷新token。start前发现其他pending flow则拒绝，不静默覆盖。Nola内部串行该owner操作，poll返回不匹配flow_id则失效、停止管理，不保存新账户材料。

官方DELETE login只接受provider，不接受flow_id或条件revision。取消前会核对当前flow_id，但无法对外部Kimi客户端提供原子CAS；界面必须说明“取消所选managed provider当前登录”，用户明确确认该范围。检测到flow变化则拒绝DELETE；不宣称能撤销已完成登录、不自动logout。OpenCode仍是来源provider/plugin各自认证，没有从其聚合auth文件推导统一OAuth生命周期。

隔离测试仅使用loopback fixture HTTP服务，检查鉴权、server identity/flow变化、错误envelope、取消和Core CAS；不访问真实Kimi服务，不发送真实登录请求。新组件独立于CodexSourceLogin，继续沿用既有Web认证/CSRF且限制local Host管理，不新增权限体系。

当前接线：`model_kimi_login.rs` 及 `KimiSourceLogin.svelte`；`/api/model-connections/kimi-login/{owners,connect,start,status,cancel}` 均在既有认证/CSRF内且仅loopback来源。owners投影允许重开管理页读取已绑定origin/server_id，绝不返回server bearer。每个origin串行在途操作；HTTP十秒、两MiB读取上限、禁止代理/重定向。POST请求UUID短期防重用，结果未知不自动重试；源端已有pending时拒绝启动覆盖。状态观察不改Core账户、不启用inference。关闭页面和超时绝不DELETE，仅停止观察；显式取消按钮另有provider范围确认并说明外部flow替换竞态。

本轮隔离验证：`cargo test -p nola_cli --lib model_kimi_login --locked` 最终3/3通过；包含重开owner观察现有flow、缺取消确认不DELETE、flow/server/account变化拒绝、HTTP200错误envelope拒绝、loopback与授权URL约束。Web `npm run check` 0 errors/0 warnings。未启动真实Kimi程序、未读取实际server bearer或OAuth材料、未请求真实登录或任何模型推理。

加入两个来源管理切片后的最新CLI lib `model_`回归为232通过/0失败/1既有忽略。直接运行测试二进制时工作目录须为`cli/`（与Cargo默认一致）；既有shell路径测试依赖当前crate下的`src/`，不能从仓库根运行该二进制后将路径假设失败归为来源逻辑失败。

### 独立 Nola ChatGPT 订阅正常调用（实现契约，集中验收中）

来源 `nola_chatgpt` 与公开线协议 `siwc_responses` 独立于Codex/OpenCode来源登录和普通 `responses` API计费。Core账户是业务权威，来源必须匹配、状态Ready、credential_ref为 `siwc:<accountUUID>`；私有SIWC会话的session_revision不是Core账户revision。目录和profile沿用现有连接/目录/选择链，不创建第二份模型选择存储。来源和协议互相约束，禁止会话覆盖改为普通Responses或指定customApiKey。

冻结Host HTTP route增加 `siwc_account`，内容为account_id/account_revision/credential_ref/client_id/subject，不含token。请求前及冷恢复必须核对当前Core账户完整绑定与私有会话client/subject、授权范围、撤销状态；合法token刷新只改变私有session_revision，不改变冻结账户身份，也不把token摘要作为订阅账户身份。普通HTTP route必须无siwc_account，订阅route必须有；订阅route的普通API credential为空且不能配置API fallback。

执行复用现有Responses流式reducer、Run attempt与usage，不另开文本聊天实现。专用适配要求stream=true/store=false，system映射developer，剔除或拒绝订阅协议不支持参数；授权材料仅由专用SIWC credential adapter交给此传输，不能进入API key字段。当前独立文本fixture只证明文本，不宣称工具/图像/推理等能力全部支持；目录能力保持声明/探测/覆盖边界。全部使用fake HTTP与隔离Core数据库，无真实授权或模型调用。

当前实现使用公开独立SIWC资源 `https://api.openai.com/v1`，不是Codex私有backend。Host invocation版本8的输出字段为 `omitted`，表示本地上下文预算仍被记录但不发送max_output_tokens。普通Run恢复与委派保存非秘密account绑定、在所属store重建私有运行context，不序列化context的数据库路径。Web/Room/辅助调用绑定同Core profile；CLI新会话通过显式 `--profile` 与精确revision选择，Mac通过同owner原生桥管理账户与冻结profile准入。Memory worker工具契约暂不支持SIWC，明确拒绝，不自动转普通API；目录能力保持未知，function namespace实现不等于供应商模型能力验证。外部gateway目录、直接目标与客户端配置导出明确排除SIWC，避免把本机注册授权伪装成共享API服务。商业/远端托管资格未验证。

### 账户主线收口审计（2026-10-19）

基本入口按owner区分：Mac API来源文件导入与独立SIWC生命周期使用本机Application数据库；CLI使用其选定数据库；Web来源登录必须在运行Host的电脑以loopback浏览器操作。iOS/Android仅提供已配对Host的API管理入口及本机来源登录handoff说明，不扫描手机CLI账户、不将Host账户复制到手机。Mac的第三方来源登录handoff也不等价于Mac本机SIWC桥。

用户仍需提供或完成：明确选择来源文件及是否导入秘密；Codex可执行文件和home；已运行且受认证的Kimi Code/OpenCode本机服务及其控制凭据；独立SIWC浏览器授权、服务端计划资格与必要scope。Kimi取消是明确确认的provider当前流程取消，OpenCode没有统一取消/刷新能力；Kimi Desktop/Work未实现来源适配。以上不能标为已支持或以读取真实凭据补救。

审计复核网关目录和直接目标共用排除SIWC的谓词，客户端配置导出也独立拒绝SIWC；来源管理统一本机同源门禁，Core/私有账户revision失效不会自动切账。未发现需新增生产修复的基本操作阻断。本次只纠正文档过时状态，未重复全量测试、未发真实OAuth/模型请求；最新Mac隔离22/22证据及Rust专项目录见 `NOLA_SIWC.md`。

2026-10-19 `cargo check -p nola_cli --lib --locked` r3通过，见 `/tmp/nola-siwc-integration-check-r3.log`。正常调用全套fixture仍在集中验收，首轮仅测试构造/新增enum穷尽分支编译报错已修，尚不写为通过。

### OpenCode provider OAuth 管理（实现契约与隔离验收）

整合安全审计（2026-10-19）：来源管理四组路由（Codex登录、Kimi登录、OpenCode登录、只读来源账户bind/refresh）统一要求POST、loopback peer、唯一Host/Origin/Content-Type。Host只允许明确本机literal或localhost，Origin必须与当前plain HTTP listener的Host（含端口）精确一致，不将localhost与127.0.0.1视为同源；缺失/null/重复Origin、DNS rebinding域名、跨站Fetch Metadata、简单表单/text请求均拒绝。本机现有Web使用loopback隐式信任，不具备远端session-cookie；此额外同源JSON门禁补足全局仅远端CSRF校验的本机缺口，不依赖CORS。远端登录session不解除来源本机限制。错误只返回静态code并no-store，来源JSON上限256KiB。Codex已完成回执也重新核对Core当前account revision，后续账户修改后不能从旧request恢复旧认证状态。

安全回归：`model_source_management_requires_exact_local_browser_origin_and_json` 1/1（13种HTTP请求与peer边界），Codex来源回执3/3；日志分别为 `/tmp/nola-source-security-gate-tests.log`、`/tmp/nola-source-receipt-tests.log`。所有HTTP都是临时fixture，合法门禁后的无效JSON在打开用户store前拒绝；未将此用例宣称为真实OAuth登录验收。

依据当前已发布[v1.19.3 provider auth源码](https://github.com/anomalyco/opencode/blob/v1.19.3/packages/opencode/src/provider/auth.ts)与[发布版OpenAPI](https://github.com/anomalyco/opencode/blob/v1.19.3/packages/sdk/openapi.json)，使用文档公开的 `/global/health`、`/path`、`/doc`、`/provider/auth` 和 `/provider/{providerID}/oauth/authorize|callback`。开发分支已存在另一协议，本轮不做双读/接口回退；连接服务缺少所需当前契约则明确拒绝。

用户显式选loopback origin、受保护服务Basic用户名/密码和来源directory。Nola只保存私有服务控制引用，Core账户为OpenCode来源且Unavailable/inference=false。OpenCode未提供稳定server UUID：owner绑定origin、请求directory、来源返回的home/state/config/worktree/directory与服务版本摘要；它是配置所有权，不是经过验证的用户或进程身份。每次操作重读owner及method声明，provider/method索引和prompts摘要变化拒绝。

认证方法由 `/provider/auth` 返回，支持oauth的text/select提示与eq/neq条件，保留api方法但不以它冒充OAuth。输入仅送所选provider/method的authorize，不持久化或回显；未知/隐藏字段与无效select值拒绝，插件内部验证仍归OpenCode。authorize返回的https授权URL、auto/code回调模式和文本说明只展示，不由Host访问。用户完成来源浏览器授权后显式确认callback；code仅本次请求传递，不进入Core/owner/日志。callback严格成功boolean后仍只报告来源完成，不启用推理。

源码的pending只按provider保存，无公开flow ID、条件callback或取消API。Nola只保证自身单owner/provider在途操作绑定，不能阻止其他OpenCode客户端替换pending；authorize/callback都须用户确认“所选provider当前登录”范围并提醒不要并行管理。关闭页面只停止本地等待，无DELETE、logout、令牌刷新或自动callback。超时/错误结果未知，不自动重试；新操作需重新明确确认。任何Nola本地attempt ID不是来源flow ID，也不伪称可取消来源授权。fake HTTP验收，不读取真实auth.json或插件token，不启动Agent session。

入口为本机Host `/models?tab=api`，正常Host认证/CSRF之外再次限制来源操作请求为loopback。`POST /api/model-connections/opencode-login/{owners,connect,inspect,authorize,callback,forget}` 复用同一Core owner数据库；`forget`只移除本机操作锁的活动状态，无来源写请求。UI在authorize发送前保留request ID，失败不提供自动重试callback；账户owner切换在pending/unknown期间锁定。Basic密码、prompt输入、code不进Core文档或公开回执；控制密码仅在既有私有vault中保存。传输禁proxy/redirect、30秒超时、2MiB流式上限。内存attempt有10分钟TTL，不声称跨重启的幂等性或源端取消；用户重开页面后必须自行确认来源当前状态，不能自动重发。

隔离用例位于 `cli/src/model_opencode_login_tests.rs`，覆盖owner/方法变化拒绝、prompt输入边界、回调HTTP200错误对象拒绝、不确定callback不重放、本地forget无DELETE与路径/loopback限制。2026-10-19 `cargo test -p nola_cli --lib model_opencode_login --locked` 2/2通过；同一测试二进制在CLI crate工作目录运行 `model_` 为235通过、0失败、1忽略，CLI模型参数filter 8/8、Web登录跳转1/1。Web check 0错误0警告（后续文案与owner切换UI改动由相应客户端验收追加）。未实际登录、读取真实凭据或调用付费/本地模型。

`cli/src/model_source_account.rs` 只支持用户明确选定的 Codex auth JSON 与 Kimi Code OAuth 凭据 JSON。公开账户保存不透明引用，私有 Host 库只保存路径/来源，不复制 access/refresh token，不写来源文件、不发登录或刷新请求。检查结果区分 `login_required`、`expired`、`present_but_unsupported`、`invalid`、`unavailable`、`disconnected`。缺失或过期提示用户先在来源应用登录；存在凭据仍明确 `inference_supported=false`，Core 状态为不可用，不把订阅等同 API 权限。JWT expiry 仅用于拒绝过期，不验证或信任其账户声明。Kimi Code 当前文件字段参见官方 [OAuth wire types](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/types.ts) 与 [storage](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/storage.ts)。Kimi Desktop/Work 不使用此适配器。

## 反向模型网关

### 普通 Run provider 用量存在性（已接线，集中验收中）

本轮发现普通Run的 `LlmUsage` 与Core `TokenUsage` 把缺失input/output/total初始化为0；Responses读取使用unwrap_or(0)，Chat/Anthropic parser补算total，累加又将reasoning加入output，导致缺失变零及潜在重复计数。修复不是SIWC特例：所有协议主计数改可选值，`None/null`表示未报告，`Some(0)`表示真实零，单独出现input/output不自动补另一个或total。缓存字段继续使用现有显式存在性，reasoning作为output细分，不加到input+output。

消费点清单：①Responses终态与Chat/Anthropic usage envelope逐字段读取，保留流式先前已报告字段，不因后续缺字段清空；②`LlmUsage::cache_usage`与provenance只使用报告字段；③普通Run与compaction `core_token_usage`传递可选值，物理attempt的usage JSON保持null，不估算成本；④CLI final-response累加和Core `TokenAccounting`按请求数区分尚无样本与未知样本，已出现缺失字段的累计该字段保持未知，不被后续已知样本填成完整总数；⑤context占用仅在input明确报告时校准，0是报告而非缺失；⑥compaction request witness保存/恢复可选计数，本地prompt估算仅用于上下文预算、不回填provider用量；⑦Core资源预算需要的累计数字单独命名为已知token下界，原始total与本地input+output派生量分开，reasoning不重复加入。

允许本地派生的唯一用途是明确标识的预算/上下文估算；它不能写入原始provider `total_tokens`，也不把部分计数当完整费用。`request_count`用于识别空累加器与未知请求；`known_token_lower_bound`与`budget_token_lower_bound()`专供资源预算，优先使用报告total，否则仅在input/output均报告时相加，不加reasoning。原始累计计数溢出变未知，本地预算使用饱和运算。不新增供应商特例或历史存储转换。测试覆盖无usage、真实全0、只input、只output、只有cache、字段分散在多个流事件、两个物理请求一已知一未知、compaction落盘恢复、reasoning是output子集。

2026-10-19 最终隔离验证：CLI `model_` 262通过、1忽略（`/tmp/nola-usage-model-final.log`），Responses 22通过、compaction 48通过（同前缀 `responses-final` / `compaction-final` 日志）；kernel用量3通过，AgentEngine context_window 13通过，Core冷恢复用量契约1通过、turn snapshot 3通过。Anthropic暖前缀fixture明确报告cache creation为0；缺此字段仍拒绝暖前缀预算，不能把未知补零。较广的agent_core_host_contract在主控范围修复后153/153通过（`/tmp/nola-core-host-contract-scope-final.log`）。此次未调用真实模型、OAuth或用户凭据。`ContextUsageSnapshot`保留既有显示投影的数字+reported，原始TokenUsage六计数为nullable，不把未报告显示为真实零；iOS独立原始用量链已同步nullable，新增用量与关联回归23/23通过（`/tmp/nola-ios-usage-presence-validation.log`）。

`cli/src/model_gateway_host.rs` 提供 `/gateway/v1/models` 和三个显式 POST 路由：`responses`、`chat/completions`、`messages`。网关使用与 Web 会话不同的独立随机 bearer token，私有凭据库 CAS 签发/轮换/撤销；不会把该 token 转发上游。外部模型身份是 `nola/<profile_id>/<revision>`，模型 revision 变化返回 409，协议不匹配返回明确错误；不换模型、不换账户、不做协议翻译、不重试。

一次网关请求恰好至多一次上游 POST，转发模型输入、工具声明、续接 ID 与 SSE 原始事件；不进入 Nola Agent 执行器，绝不执行外部客户端的工具。只透传白名单协议头，供应商凭据从精确路由的 Host vault 获取。最多四个活动请求、32 MiB 请求体、10 秒连接及 300 秒请求时限、禁止重定向；已知网关 origin（含 loopback 别名）拒绝回环。响应提供独立 `X-Nola-Attempt-ID`，流错误不拼接新尝试。下游放弃响应将释放上游 reader；尚不声称阻塞 HTTP 栈具备独立主动取消接口。

`model_gateway_usage.rs` 在上游请求前经 `RecordGatewayAttempt` 记录 `in_flight`；初始账本写失败则不发送上游请求。账本以 `model.gateway_attempt.<id>` 独立 revision 保存，绑定 profile/revision、connection、account 和已认证网关 token 代次 `client_access_revision`。后者表示共享网关凭据主体，不伪称能分辨使用同一 token 的不同应用安装。该键空间禁止 generic product setting 写入，终态不可重写，不存在会话 ID 或会话累计副作用。

JSON/SSE reader 附带有界 usage 观察器（最多 2 MiB 单事件/JSON），不改变透传字节、不持久化提示词或输出正文。Responses/Chat/Anthropic usage 字段各自保持可选值；重复累计快照取已观察最大值而不是相加，缓存/推理细分不重复计入主 token 数。缺少 usage 保留 `null`，部分字段缺失也保持 `null`，不估算价格或补零。SSE 只有观察到完成事件且流正常 EOF 才记 `completed`；缺终止事件、读失败或下游丢弃记 `interrupted`，HTTP/事件错误记 `upstream_rejected`，发送失败记 `transport_failed`。进程崩溃或终态持久化失败保留 `in_flight` 表示结果未知，不自动重试、不冒称失败且未计费。只读 `ListGatewayAttempts` 支持有界分页。

## 外部客户端配置写入

`cli/src/model_client_adapter.rs` 是与导入完全独立的写路径。明确选择已存在的绝对配置文件后，生成十分钟私有计划；公开预览只有来源、路径和将新增的字段，不含 token。确认重新检查完整源文件、网关 token revision 和模型 revision，再以同目录 0600 临时文件原子替换。JSONC 与 TOML 使用具体语法树保留无关字段/注释。进程内写串行化；最后重读至 rename 之间不宣称提供针对任意外部编辑器的跨进程 CAS。

| 目标客户端 | 新增的 Nola 自有字段 | 默认选择 |
|---|---|---|
| OpenCode | `provider.nola-<protocol>`，对应 SDK、网关 URL/独立 token、revision 模型 ID | 不覆盖用户默认模型 |
| Codex | `model_providers.nola-responses` 与 `profiles.nola-gateway` | 用户显式选该 profile；只支持 Responses |
| Kimi Code | `providers.nola-<protocol>` 与 `models.nola-gateway`，沿用明确上下文限制/能力 | 不覆盖 `default_model` |

已有同名字段拒绝覆盖。确认结果保存私有 ownership receipt，公开列表可跨重启查得 receipt_id；断开仅在 Nola 自有字段与 receipt 相等时移除这些字段，保留用户并发新增的其它配置。若用户改了 Nola 自有字段则拒绝并提示人工处理，不回写整份历史文件。原子写前先保存 receipt，失败时移除未应用 receipt；崩溃窗口保留可检查的 receipt 而不自动回滚来源文件。网关 token 可独立撤销，不删除用户来源登录或任何会话数据。

以上代码与隔离 fixture 测试已经编写；是否通过编译、真实客户端接入和流式调用验收以集中验证记录为准，不能把未运行测试写成完成证据。
# Apple 来源导入的存储所有权

## 显式能力验证的实施边界

首个主动验证只验证精确模型/协议的最小文本生成，不测试工具、图片或推理参数；这些未测试项保持 unknown。Host 接收显式确认（账户 ID，包括匿名 null、账户 revision、凭据 binding、connection revision、模型 ID、协议、最大请求数固定 1、可能计费确认），发送一次最小 POST，不重试，不自动切协议，不执行返回工具。取消复用 request UUID 与目录操作状态机，取消不保证供应商未计费；结果仅在原 revision 尚有效时提交。发送前在 Core 主库 model.probe_attempt.* 专属记录中事务保留 request UUID；重启、超时或结果未知均不允许重放该 ID，重新发送必须由用户重新确认并使用新 ID。记录不含提示、响应或秘密，不使用额外文件账本。

目录的 execution_capabilities 保持来源声明；连接保存独立 capability_assessments，记录文本成功证据和可选的用户 execution override。证据不伪造其他能力，覆盖不改写来源声明，地址/账户/协议变更清除对应证据与覆盖。用户覆盖优先于来源声明；未声明工具能力时采用 Core 的导入默认值（toolCalls=true），明确的 provider 声明（包括 false）覆盖默认值，用户覆盖具有最高优先级。这是产品默认配置，不是文本探测证明了工具支持。失败/限流/取消不写 false 证据。证据包含模型、协议、原 revision、请求 UUID、观察时间与请求数，不保存提示、响应或秘密。

CLI 的 `nola model probe <connection-id> <revision> <model-id> <protocol>` 必须使用交互终端；先打印精确确认对象，再以默认拒绝的 y/N 提问，非 TTY/JSON 模式拒绝，不提供自动确认的付费开关。`nola model override <connection-id> <revision> <model-id> <protocol> <clear|capabilities-json>` 是不发网络的显式配置命令：JSON 必须声明 toolCalls、imageInput、reasoning、samplingParameters，未知字段与矛盾限制拒绝；clear 恢复声明、保留历史证据。所有写入均通过当前 revision CAS，不改变已选模型。

移动端通过已授权 Host 管理页入口使用同一确认流程，明确账户属于选中的 Host，而非手机。Mac 本机扩展现有 model-import-bridge 的 probe_prepare/probe 操作，与本机 Application 使用同一数据库，Rust 仍拥有协议请求、持久 attempt 与证据提交。Swift 服务只响应精确连接/账户/URL/协议/nonce 绑定的私有 credential_read_request，直接读取 Keychain，不将秘密交给 View/receipt/debug。Mac Keychain 无 Host vault revision，因此绑定当前账户 revision、精确路由和当前服务读取材料的 opaque digest；确认前、发送前、提交前重读校验，不能将 Host 引用当成本机凭据。取消终止本次私有子进程，已经发送的请求可能计费，未知 attempt 永不自动重放。本机目录 GET 不会隐式转成付费验证。订阅 OAuth 登录、刷新、撤销仍不在此实现内。

macOS 模型账户页通过显式文件选择、共享 Rust 预览和确认导入，不扫描来源目录。现有 `nola-agent-host` 的 `model-import-bridge --database` 私有服务模式使用 Mac Application 的同一数据库；复用 Host 的来源适配器和提交事务，不引入 Swift 解析器或第二套账户存储。凭据与环境读取分别征得同意，默认关闭。

导入服务以 `ImportCredentialSink` 注入凭据所有者。CLI/Web 使用 Host vault；Mac 私有管道的凭据请求仅由服务层消费，验证 request UUID、nonce、已确认候选的 connection、endpoint、protocol 和 account identity 后写入现有 Keychain，返回不透明 `account:` 引用。秘密不进入 UI、Core 回执或日志；子进程 stderr 丢弃、帧大小和等待时间有界。提交失败不删除原凭据，已写入的新槽可留待后续回收。预览与确认保持同一进程，来源变更须重新预览。

iOS 不扫描手机上的 CLI 账户。用户显式选择已配对 Host，校验 Host identity 后，通过现有短时浏览器管理授权进入 `/models?tab=api` 完成来源预览与确认。文件、凭据和导入结果归该 Host，不宣称同步为 iPhone 本机账户。未配对时进入现有 Host 连接流程。



### 导入默认值（2026-10-20）

Core `ModelExecutionCapabilities::for_import` 集中定义工具默认值：缺少声明为 true，显式 true/false 原样保留。
CLI/Host 的 provider 元数据适配复用该规则；Web、Mac、iOS、Android 的新建和能力编辑草稿与此一致。
打开编辑器优先使用用户已有设置及 provider 声明，不以空白草稿覆盖明确关闭。
运行时缺失或损坏的完整能力声明不因此获得执行资格；工具权限审批也不受此默认值影响。
不扫描、迁移或批量修改已有用户模型。

### ChatGPT 目录与推理档位（2026-11-04）

ChatGPT 目录的 `supported_reasoning_levels` 是产品选择项，不等于 Responses 的单模型 `reasoning.effort` 枚举。`ultra` 是多代理模式，原始目录观察保留它；共享 Application 在生成 `siwc_responses` profile 时仅排除该模式，其他未识别 effort 仍准确拒绝。Nola 不把 Ultra 改写成 max，也不因此启用或承诺多代理模式。每个模型仅使用其声明的单模型档位。

Mac 必须读取 Core 的稀疏 reasoning 声明；本地编辑器的 responses/budgets/outputLimit 默认字段不是目录启用的必填条件。`responses` 和 `siwc_responses` 共用已有 Responses transport 判断，账户身份及凭据解析仍各归原有 Host。

授权页区分浏览器授权、保存账户、发现并启用模型三个步骤。保存成功留在结果页并给出模型设置入口；打开浏览器、保存和取消均由用户明确操作，不以目录成功冒充模型调用成功。Ultra 的能力边界显示在账户连接的模型列表。

底层显式 agent tier 的 `siwc_responses` 解析与 Responses 共用受支持 effort 语义；这不规定聊天必须使用三档，聊天原生菜单保留完整声明列表。不得套用 Chat Completions 必须具备 low/medium/high 的条件。只有目录声明的普通 effort 可进入请求，`ultra` 仍不是单模型请求值。
