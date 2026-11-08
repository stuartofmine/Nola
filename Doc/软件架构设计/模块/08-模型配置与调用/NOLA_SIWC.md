> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；不实现旧版本兼容、双读双写或历史数据迁移。不匹配的格式明确拒绝，不自动删除、重建或改写用户数据。当前会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola 独立 ChatGPT 计划授权与推理

2026-10-19；本文件为实施设计，不表示真实授权/推理已验收。

## 官方资格与独立身份

已实际读取官方 [quickstart](https://developers.openai.com/siwc/quickstart)、[OSS 注册](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)、[账户生命周期](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)、[模型/推理](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)、[预览限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)、[OIDC 验证](https://developers.openai.com/siwc/website)。公开 OSS 流程不要求预发 client secret/API key；首次动态注册入口为官方指定 `dynamic_agent_client`，真正 client ID 由用户同意后的 callback 返回，后续复用。Nola 仓库 Apache-2.0 许可仅支持按本机开源工具范围实施；不是商业/公共远端托管服务接入资格证明。真实用户计划资格、发布方式与服务端授权均未验收；商业部署需另获官方许可，不开放公共多租户共享凭据。

## 所有者与安全不变量

新模块 `model_siwc` 与 Codex/OpenCode/Kimi 来源管理完全独立。不扫描它们的文件，不接受导入第三方 OAuth token，不启动 Codex Agent。凭据所有者为本机 Nola 的稳定 host UUID + 经验签的 issuer/subject + 动态注册 client ID；email 只作标签，不能合并账户或 workspace。host ID 首次生成后持久化，重启/退出不重建。新登录完成前不替换任何活动账户。

Token 与 pending PKCE 材料只进入 Host 私有凭据库，不放 Core、浏览器存储、URL诊断、usage、日志或公开快照；公开投影仅账户标签、registration ID、状态与 revision。不以可伪造的通用设置作为认证权威。后续 Core profile 必须显式标注 SIWC transport，不把 OAuth token 写为普通 API key；源身份或 transport 不匹配直接拒绝。

## 授权、刷新与退出

1. 在 `127.0.0.1` 随机端口启动短时 callback listener 后生成 state/nonce/PKCE；callback 固定 `/auth/callback`，后续仅端口可变。授权 URL 使用官方 auth endpoint，首次 app 名为 Nola；旧 registration 不再发送名称，不混用其他账户 hint。
2. pending 绑定账户/host、原 URI、state、nonce、verifier、client ID 与十分钟截止。callback 的重复字段、错误 state、过期、错误client ID均拒绝；一次性消费后才换码，access_denied不换码。不得将动态注册占位 ID拿去换码。
3. form POST token endpoint，带原redirect/PKCE/resource，不使用client secret。用成熟 JWT 库验证 JWKS签名、算法、issuer、issued-client audience、exp/iat/nonce；返回账户与旧subject/client不符拒绝。只信token响应的granted scopes，不信callback的scope。
4. 同会话的刷新/退出通过进程外锁串行，持锁重读原凭据revision。换新access/refresh/scopes/expiry整体CAS；发出刷新前持久化in-flight标记。崩溃/网络不确定后不能盲重放旧rotating refresh token，应重新授权。不同账户锁独立，不静默换账户。
5. 退出先阻止新请求，再按OIDC discovery的revocation_endpoint提交原refresh token和client ID。200（包括空body）确认远端撤销。失败后本机清除仍完成，必须保留remote revocation未确认状态并指引到ChatGPT设置撤销；不能在已清除token后声称本机可重试撤销。保留registration/identity/host元数据，不删除其他账户。

## Responses transport

仅固定官方 `https://api.openai.com/v1/models` 与 `/responses`。目录使用选中账户token，解析`models`数组，显示`visibility=list`的display_name并保留原顺序，发送精确slug。不是普通 API key 目录的`data`形状。HTTP禁止跨源重定向、限制超时/单帧与总响应大小；错误不回显token/body。

SIWC请求显式构造`stream:true, store:false, input:[...]`，完整发送必要历史；不传previous_response_id/max_output_tokens/temperature/top_p等官方不支持字段，不用自动删字段掩盖调用方错误。本地 function 工具采用下述 namespace 适配并复用 Core 权限执行；不承诺 custom 或未验证模态支持。stream必须观察response.completed；failed、incomplete、断流分别报告，不用EOF当成功，不重试/跨账号/转API计费。usage保留可选值，由现有物理attempt账本接入，不伪造0。

## 实施与测试边界

先落独立协议/私有会话模块与fake loopback HTTP：state/PKCE、真实JWT验签失败、同email不同subject、scope缺失、rotation/并发/崩溃未知、撤销失败、目录形状、SSE终态/usage。production URL与fixture注入分离，测试不能把任意HTTP作为生产issuer。没有真实OAuth或模型调用。

完整交付还需本机显式登录/账户选择UI、Core账户/transport类型、既有Runner冻结/恢复/usage接线和端到端fixture；仅协议单测不等于五端或订阅推理已交付。未获授权的远程商业部署、移动端本机callback能力不能从桌面流程推定支持。

## 本机 CLI 生命周期（接线前设计）

- `model chatgpt sign-in <label>`：本机 TTY 默认拒绝的明确授权确认，生成新账户UUID；打印仅此次系统浏览器授权URL（不含token hint），真实loopback callback完成后才SaveAccount。不是推理命令，不自动请求目录或付费验证。
- `reauthorize <account-id> <core-revision>`：仅原NolaChatgpt来源/凭据owner，复用原client/subject/host；新验证subject不同拒绝，不把同email当同账户。
- `status <account-id>`：只读Core账户及私有会话的脱敏状态；不联网、不自动刷新。`models`、`refresh`、`sign-out`各要求原Core revision；目录不推理，刷新不选模型，退出确认后仅清原账户材料。
- Core账户是唯一业务列表和启用权威，vault session_revision仅约束私有材料更新，公开字段分名。Core source/opaque ref不匹配时不读该槽。跨库写失败明确报告；私有槽仍需原Core owner和状态匹配才可运行，不能从vault“已登录”独立推导运行许可。
- 正常会话接线由现有Responses transport/reducer/physical attempt/usage路径承载。模块`infer_text`限测试编译，不添加第二个聊天命令。专用transport须冻结到请求/恢复描述，禁止被普通API key路径或跨账户fallback消费。
- 目录接入采用既有Core连接/catalog链：显式`models`读取并同步`siwc-<accountUUID>`独立连接，协议仅`siwc_responses`，目录未报告execution capabilities时保存None（未知），不制造全false/全true，不自动启用模型；用户通过既有`model configure`以返回revision和精确catalog key启用/选择。目录请求后再次校验Core owner，连接已有其他owner时拒绝，不覆盖。

能力分三层：服务目录事实（可未知）、适配器已实现的协议能力、用户对具体模型/route的明确覆盖不能混为一谈。既有`model override <connection> <revision> <model-id> siwc_responses <capabilities-json|clear>`允许明确声明toolCalls；Core以user_override独立存储并投影到profile，再由原Runner模型能力读取。未声明时configure使用现有保守文本baseline且保留目录未知，不标成provider已确认不支持。覆盖不代表真实验证、不改变工具权限；clear撤回覆盖，回到当前目录/保守baseline。

## 正常会话私有凭据边界

### Mac 本机非 TTY 生命周期桥（已实现并完成隔离原生验收）

Mac 使用已验证的现有 runtime executable，以 `model-siwc-bridge --database <本机Application同一绝对DB路径>` 启动私有 JSONL 服务，不解析TTY输出、不启动第二部署二进制、不把配对远程Host账户当Mac本机账户。Rust复用同DB的SiwcHost、稳定host UUID、私有vault及账户/session CAS。Swift只接脱敏状态与一次授权URL，不能读取/传递OAuth token、PKCE或第三方应用凭据；child stderr不进入UI诊断。

`hello`返回本进程owner nonce及协议1；所有后续请求绑定该owner。`start`要求新UUID或现有NolaChatgpt账户UUID、精确Core revision（新账户null）、显示名和明确授权确认。服务立即返回新flow UUID，后台调用既有begin_sign_in/finish_sign_in，stdin保持可响应取消；不等待浏览器回调才返回start。`status`只读此flow，阶段为preparing、awaiting_authorization、authorized、failed、cancelled、committed。只有awaiting_authorization暴露一次流程URL，Swift必须在用户明确授权后交系统浏览器，不能记录URL。`cancel`与`finish`必须同时匹配owner/flow/account/revision；未知或迟到的另一账户flow不能改变当前账户。

后台只处理官方OAuth及私有session，授权成功后先停在authorized；`finish`重新核对Core原revision、来源/ref及私有session revision/subject/client，再执行既有SaveAccount。失败不回滚或删除原凭据、不宣称登录完成；返回可区分的Core提交冲突/私有session已变化状态。取消只能停止本机等待，不能宣称撤销已授予的远端权限；若取消与换码完成竞态，明确私有授权可能已保留，Core不自动提交。EOF/close取消全部未结束flow，后台不得在关闭后提交Core。每个进程最多一个活跃flow，终态保留到进程退出，不自动重试/覆盖不确定流程；新登录需要新的显式操作。

帧以request_id UUID关联，最大64KiB，固定结构化错误不包含provider body、stderr、token或完整URL。进程owner nonce不替代OS管道/文件权限，只用于拒绝跨进程/旧UI回执。测试用假issuer/假callback和隔离vault检查及时start、精确cancel、EOF、跨account/revision拒绝、已授权后Core CAS冲突及重复finish幂等，不运行真实OAuth或模型请求。本设计不是已验证完成声明。

同桥补全`account_status`、`models`、`refresh`、`sign_out`，绑定owner/account/Core revision；网络动作另绑定上次只读status返回的私有session revision。status不联网；models只GET目录并复用CLI sync_catalog写回同Core连接，不自动启用/选择、不制造能力事实；refresh只显式调用原刷新，sign_out额外要求明确确认且复用原撤销/保留revocation_unconfirmed语义。刷新或退出后私有会话与Core更新之间持账户锁再做CAS，冲突明确报告私有会话已变化、不得删除原数据或自动重试。Swift顺序发请求，网络操作有原Host超时；关闭进程不能被宣传为撤销已发送的刷新/退出，用户必须重新读状态。返回account/session/connection均为原脱敏结构，不让Swift使用API key目录或推理替代SIWC专用契约。

2026-10-19：`cargo test -p nola_cli --lib model_siwc_native --locked` 4/4通过，日志`/tmp/nola-siwc-native-management-tests.log`。覆盖假OAuth回调、授权后取消不提交、重复finish、Core冲突、私有会话保留、双revision目录/刷新/明确退出和远端撤销未知、EOF与帧上限。无真实OAuth/推理。Mac Swift服务/UI和正常Run admission的同owner profile接线仍需集中验证，不能以此4项宣称原生推理已验收。

随后同owner正常Run接线的集中Rust验证：`model_siwc` 20/20、`native_host_request::` 23/23、`cold_host_` 2/2通过，分别见`/tmp/nola-siwc-mac-owner-final-tests.log`、`/tmp/nola-native-host-request-final-tests.log`、`/tmp/nola-native-cold-host-final-tests.log`；包含Mac精确profile+revision准入、不接受普通API key、专用账户绑定与原冻结恢复。Host Mac descriptor schema测试1/1通过。Mac最终隔离原生验收22/22、0失败，日志`/tmp/nola-mac-siwc-native-validation-r4.log`，覆盖账户桥、授权迟到、profile准入、凭据边界与恢复；没有真实订阅账户或模型服务验收。

Mac正常Run的冻结launch descriptor另携带必填nullable `model_profile_revision`：SIWC使用精确正数revision与profile ID，普通路径可显式null；缺字段拒绝。冷恢复必须读取原冻结revision并重验同owner账户，不能用当前profile revision重构不同配置。

可信 admission 调用 `execution_binding`，只冻结 Core account ID/revision、`siwc:<UUID>`引用、client ID及subject；不冻结旋转token或vault revision。每次 Responses dispatch 通过 `SiwcExecutionContext` 的原数据库重读 Core exact owner/state/revision/ref，再验证私有issuer/client/subject。临期刷新在同一账户跨进程锁内执行，随后再次检查Core。专用credential对象只向Responses适配器提供bearer/resource并持锁至消费结束，不写Runtime普通API key。账户断开、身份改变、刷新未知均明确拒绝，不能静默重capture或fallback。

## CLI 新会话显式 profile 选择

新增 `nola run --profile <Core-profile-id> --profile-revision <revision> <text>`，也可不带text进入原交互终端。这是一对只在命令行消费的选择意图，不反序列化自Config、不含token，不使用全局selected自动切provider。只用于新持久会话；与endpoint/model/API key参数、resume/continue/session/no-save/Host stdin冲突时拒绝。读取enabled且非deleted的精确Core profile/revision，沿既有session override、snapshot_for_settings、bind_siwc_profile和restore路径生成正常配置。提交时仍走原Runner/attempt/usage；SIWC api_key为空。已有会话保持其canonical配置，不能用新参数静默覆盖。目录输出profile可达链保持显式configure后再选择。

`model connections` 同时列出脱敏profile ID/revision/名称/启用状态/所属connection/protocol，供用户复制精确选择参数；不输出完整source JSON或凭据引用内容。完整路径是sign-in→models→configure→connections→run --profile，并无额外推理命令。

## SIWC 本地 function namespace 适配（协议层）

## Mac 原生 Host 发送与冷恢复接线

Swift只识别siwc_responses并提交Core profile ID/精确revision，不走Keychain API账户解析，不传api_key/fallback。stdin顶层model_profile_id+expected_profile_revision必须与冻结mac_launch.model_profile_id/model_profile_revision一致。Rust在Mac可信stdin边界读取同一DB精确enabled profile，复用resolve_remote_profile_settings+bind_siwc_profile，账户身份仅由Core和私有vault解析。模型能力以Core profile为准，Swift不能覆盖声明；运行时保留既有工具权限/Host资源配置，正常Runner/attempt/usage不变。

冷恢复保留原descriptor profile revision及原HostInvocationBinding.siwc_account，不重capture revision。恢复构造出的账户identity须经既有HostInvocationBinding精确比较后才允许执行；profile/账户变更明确拒绝。Swift不读取/复制OAuth token或凭据摘要；RustResponses在dispatch前才解析私有token。

冻结请求若指定 `tool_choice:none`，保留原工具 schema 以维护前缀，但拒绝任何新工具调用，不能仅凭名称在目录中就执行。历史调用仍可回放，不视为当前执行授权。

2026-10-19重新读取官方[preview限制](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)、[function calling](https://developers.openai.com/api/docs/guides/function-calling)及[Responses类型](https://developers.openai.com/api/reference/resources/responses/methods/create)。采用顶层namespace包装现有Core function schemas；不启用hosted tool_search、programmatic_tool_calling或additional_tools动态装载。custom虽属服务协议允许范围，本轮Core没有对应自由文本执行契约，不伪装成function。

`FrozenSiwcTools`只做无副作用协议映射：从已准入Chat function schemas生成固定`nola` namespace，保留名称/description/parameters/strict，未指定strict时显式false避免服务隐式收紧schema；拒绝重复名字、非法名字及defer_loading。冻结body可重建同一精确映射。响应added和completed均以独立(namespace,name)二元组查询本次冻结catalog；未知、缺namespace、前后身份改变都拒绝，不取点号后缀、不按提示词路由。映射返回Core原canonical name，执行/权限仍由既有Core负责。

历史中真实provider function_call保留独立namespace/name/arguments/call_id及原始其他字段，不删namespace、不把已不在当前catalog的历史记录当新工具授权。由Core正常tool_calls投影重建的项仅通过显式encode接口补确定的`nola` namespace；function_call_output以原call_id关联，原有namespace/name如存在必须合法一致。本轮只支持function，不把custom/hosted项偷偷改写。fixture覆盖schema、冻结body、未知namespace与同名混淆、多语工具description/参数原样、历史已撤销工具、added/terminal调用解析。完整Responses工具往返、Core权限与usage回归未完成前catalog仍不声明已验证工具能力。
