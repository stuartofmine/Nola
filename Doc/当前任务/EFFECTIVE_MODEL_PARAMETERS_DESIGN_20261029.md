# 原生稀疏模型参数：共享有效值查询设计

状态：**2026-10-29 Core 查询源码已主审并通过受影响编测；平台参数 caller 迁移尚未实现**。不修改当前正在验收的 Android/iOS runner，不把 raw profile 或模型选择结果称为有效参数。

## 1. 已确认的问题

`android/.../data/repository/ConversationParameters.kt::readParameterFields` 要求 `maxContextLength/maxOutputTokens/temperature/topP` 四项同时非空，否则返回 null。合法覆盖 `{temperature:0.2}` 会整体丢失：普通发送拒绝“参数缺失”；加载显示 `ModelParameters()`；专家工厂和选型会退回模型默认，忽略 0.2。`null` 作为移除物理标量覆盖也同样被当成整份参数不存在。

iOS 的 `ConversationParameterSnapshot.read` 有相同四项要求并抛错。`applyingOverrides(to: profile)` 只在专家路径部分解决读取，仍是 Swift 合并；Mac `MacConversationModelOverride.applying(to:)` 也各自合并。不能再给 Kotlin 加一份 profile defaults 合并来补洞。

## 2. 可复用的唯一 Core 实现与真实缺口

`core/nola-application/src/model_profile_resolution.rs` 已有：

1. `profile_with_session_contract(profile, overrides)`：更新当前 protocol/reasoning 声明，不能把覆盖协议与默认协议混着校验。
2. `session_override_request(overrides, reasoning, protocol)`：缺失／null 的四个物理标量不进入 request；保留数值 0；seed 使用当前 typed `clear/value`；stop 与 generation options 使用现有字段语义。
3. `resolve_remote_profile(profile, request, tier)`：对 profile 默认、request、能力与 protocol 统一解析；返回 exact `HostModelProfileReference` 与 `ResolvedModelRuntime`。

CLI 的 `cli/src/model_profile_resolution.rs` 现在 re-export 前两项，并在远端解析中使用第三项；Web 实际请求、普通 CLI 会话、专家 native 工厂可以继续共享这一实现。

目前这些是 Rust library 函数，`ResolvedModelRuntime` 不是现成原生 JSON 查询 DTO。没有发现 Application/通用 ABI 暴露有效参数查询；`Model.GetProfile` 返回原始参数，`ResolveChatModel` 只解析选择，`ResolveModelReasoning` 只解析 reasoning，均不足以替代有效快照。无需新增独立 FFI 技术栈，接现 Application query/Swift/JNI/Service 通道即可。

还要在同一实现补两个边界，不能靠客户端补：

- `resolve_remote_profile` 当前 base snapshot 对四个 profile 参数使用 `get().cloned().unwrap_or(default)`；如果 current profile 允许显式 null，该 null 会进入物理 request 并被拒绝。应按当前字段契约在 Core 统一决定 profile-null 的继承策略，并与 override-null 分开测试；不要只修 Kotlin 后仍让 Core 路径分叉。
- 当前函数明确只接受 runtime=`remote`。本机模型的物理 context/window 上限与 Host runtime 默认值尚不能冒充远端 profile 默认。需要 Core 接现可信本机 route/profile 能力声明形成明确 baseline；无声明返回 `unavailable`，不让原生端猜 local 默认。远程绑定会话也必须由其源 Host 解析，不能借当前设备默认。

## 3. 最小查询契约（拟议，尚不存在）

新增一个只读 `ResolveEffectiveModelParameters` 用例，同一个 consistent-read snapshot 读取当前 conversation settings/configuration revision 与 exact model profile/revision。不要从 renderer 接 profile/source_json/body 或任意“已解析默认”当执行依据。

输入按用途闭合区分：

- `current_conversation`：conversation ID、expected configuration revision、expected model profile reference；Core 校当前选择／继承解析仍对应这个 profile。
- `candidate_preview`：exact profile reference，以及可选当前 conversation/configuration revision；仅为选择/参数编辑展示，不声称当前会话已选该模型，不自动落盘。

返回最小 DTO：

- conversation/configuration revision（无会话时显式 absent）；原 exact profile ID/revision；当前 canonical model key；用途标记。
- 当前有效 protocol、typed reasoning selection/tier、sampling 标记、物理 temperature/topP/output/context 值和现 `GenerationOptions`；保留 effective-null/absence 的字段语义，不能把 optional null 变成 0。
- 原 canonical sparse override 投影及其字段状态，用于界面区分“覆盖/继承/显式 clear”，不把有效值自动回写为覆盖。
- 独立 native presentation/guidance 投影；它不改变 generation baseline，也不授权工具。需要 custom instructions 时仅返回本次本地可信 provider 已可读取的当前配置字段，不返回 credentials、credential reference、完整 source_json、账号或 endpoint 私有材料。

Core 的当前全局默认模型/profile 改变、模型参数 revision 改变、configuration revision 改变时，旧查询 token/pin 明确 stale；UI 重读并保留编辑草稿，不偷偷用新默认发送。查询返回值不是 BeginRun 授权，原 actual admission 仍重核 exact pin/配置和能力。

专家避免再增加一次不一致读取：现 `PreviewExpertTemplateCapture` 内的 participant 可附加同一纯 resolver 产生的有效参数 DTO；保留 `model_declaration.parameters` 作为原 raw profile 事实，**不要覆盖为合并结果**。actual normal expert 使用原 manifest/context/revision 所绑定的同一解析；编辑重发继续使用原 FrozenResendModel/typed instruction witnesses，不重读当前默认。确需已冻结专家查询时，走现私有 Host factory 的 exact(context,run)+capture authority，而不是让公开查询接任意 frozen JSON。

## 4. 所有已定位的真实调用点

| 当前 caller | 现行为／应接线位置 |
| --- | --- |
| Android `ConversationRepository.kt:228 modelParameters` | 普通 native 有效参数主读取入口；改调用共享查询并机械映射结果，不能 null 后构造默认 |
| 同文件 `saveModelParameters:240–250` | editor baseline 比较目前读同旧 parser；应比较 query 的 exact profile/config revision 和 baseline token。保原 CAS，不用 raw sparse 与完整值直接比较 |
| Android `ChatViewModel.kt:1304 loadConversation` | 当前 null→`ModelParameters()` 隐藏问题；展示 current query 结果或明确 unavailable |
| 同文件 `:2245` 模型选择 | 当前 null→`modelParamsStore.loadParameters`；candidate query 取代该继承分支 |
| 同文件 `:3333` actual ordinary native turn | 当前 checkNotNull 整快照；用 current query/pin，准入仍校本次 config revision |
| 同文件 `:669` normal expert capture | 当前 sparse parser 失败就 raw declaration fallback；用 Core preview/manifest 的有效参数，不丢 partial override |
| Android `data/local/AndroidExpertHostFactory.kt:161 userDefinitions` | 负责实际 native custom instructions；使用相同有效 generation + 独立 presentation，不自己选 profile/default |
| Android `ConversationParameters.kt::readHostParameters`、`ConversationRepository.modelParameters/saveModelParameters` 的 WebHost 分支 | clientHostParameters 也可能 sparse；交源 Host 解析，未连接显示不可用，不本机补默认 |
| iOS `ConversationStore.swift:346 selectConversationModel` | 当前已存在部分覆盖也强读四项；不为“补齐”而重写已存覆盖 |
| 同文件 `:427 conversationParameters`、`:436 hasConversationParameters` | effective read 与“是否存在 override”分开；has 不再以能否解析完整参数充当存在判断 |
| iOS `ChatManager.swift:2990 generationParameters` | 普通真实发送参数入口，接 shared query，保持来源/config pin |
| iOS `IOSExpertTemplateHost.swift:23 applyingOverrides` | 目前原生合并入口；改消费 Core preview effective DTO，typed definition producer只负责原生展示/指令成分 |
| Mac `MacWorkspaceModel.swift:1637 defaultModelParameters/:1651 effectiveModelParameters/:3892 actual run` | profile + Mac override 合并目前在客户端；迁移到相同 query，当前 style/guidance 与数值配置继续分离 |
| Mac `Models/MacReasoningConfiguration.swift::MacConversationModelOverride.applying` | 共享替代后删除该业务合并；保留 native editor presentation，不维持双 resolver |
| CLI `conversation_invocation.rs`、`web_parts/conversation_configuration.rs`、`native_host_request.rs`、`model_file_config.rs` | 现已用纯 Core funcs；在新 wrapper 抽取时继续用同实现，不能因新查询再复制一套解析 |

桥接需要同步新增 Application command、read-only 分类与一致读 bound、Runtime/Service 既有执行入口、`AndroidApplicationRuntime.kt`、`IOSApplicationRuntime.swift`、`MacApplicationPort.swift` 的薄 DTO。当前没有理由增加新的 C ABI 符号或平台数据库。

`writeConversationParameters` / `ConversationParameterSnapshot.updating` 当前是显式编辑保存的写入口；查询不得调用它们补齐已有 sparse JSON。后续若支持只改某一个参数，沿当前 typed patch/selection 命令保存用户明确修改字段，保留其它 omitted/null/clear 原义；不能把 UI 显示用的 effective DTO 整份自动覆盖回存储。

## 5. 下一批验收门槛

共享 resolver 的实际 Store 回归至少覆盖：仅 temperature、仅 context/output、null 继承、数值 0、seed omitted/clear/value、stop empty、protocol override 后 reasoning 校验、optional generation clear、profile revision 和 context revision 读取竞态、unsupported/local source声明缺失。断言 resolution 不改变原 settings/profile，也不读凭据。

原生测试分别从真实 sparse settings 经过上述 actual caller 取得相同有效参数，至少用一个当前真实配置模型实际发送普通消息和带 custom/style 指令的专家消息；核最终 FrozenResendModel / Begin config 与 query witness 一致。Full snapshot roundtrip 测试不能代替 sparse/null/clear 的执行验证。

此文不宣称 local/WebHost 未实现的 baseline 已可解析；实现时必须覆盖或明确阻止这些来源，不能保留“遇到 null 回客户端默认”的当前行为作为兼容层。


## 6. 当前 Core 源码批

实际入口为 `ApplicationCommand::ResolveEffectiveModelParameters { input }`，JSON type=`resolve_effective_model_parameters`。`input.scope` 闭合为 `current_conversation`（会话 ID、expected configuration revision、expected model profile）和 `candidate_preview`（exact model profile、可选 conversation pin）。Profile pin 使用现 `HostModelProfileReference {profile_id,admitted_revision}`，不新增另一种版本字段。

输出 `EffectiveModelParametersSnapshot` 包含用途、会话 pin、nullable `verified_model_profile` / `model_key`、原 `sparse_overrides` 与独立 `client_model_presentation`、typed `resolution`。Available 仅含 protocol、四个 generation 标量、typed reasoning、generation options、sampling 与输出 wire 字段。不含 endpoint、账号、credential reference、source JSON。这里的 context window 是请求配置，不是实际 Host prompt budget/工具授权。

原专家专用 immutable catalog/default getter 已移为 `ModelResolutionFactsRepository`（同 Store 连接只读事实），专家 repository 继承复用；没有第二个 catalog、setting store 或缓存。查询和专家 preview 复用 `resolve_profile_parameters`，原 `model_declaration.parameters` 不改；preview 新字段为 `effective_parameters`。中央 family/read-only/dispatch 已接，入口使用 consistent-read，未调用保存、准入、默认写入或 receipt API。

实际存储 profile 对 raw JSON 仅校 JSON 形状，因此可存入 base scalar null，但原物理 resolver 会拒绝。这批保持该拒绝，不把存储可表示等同于可执行。会话 null 则按现继承语义：pure helper 已排除 wireProtocol/reasoning/seed/reasoningPreference 的 null；typed seed.clear 仍明确清空；optional generation null 仍明确清空。没有向客户端增加合并器。

当前有效来源仅涵盖现 remote profile 纯解析器。本机实际 runtime `mlx` / `llama.cpp` 返回 `local_baseline_unavailable`，仍核本机 profile pin；现 `agent-host`、远端 source-owned conversation、本机会话的远端执行 Host binding 返回 `source_host_required`，不回显输入 profile 为已验证源身份，不虚构 local model key。坏 pin、stale 配置/profile、当前选择不符、坏参数/能力仍硬拒绝。

**这些 unavailable 是尚未完成的权威 baseline 接线，不是产品能力删除**：现 Apple/Android local 与 WebHost 实际发送路径保持不变；下一平台批须补可信来源，再删客户端参数合并。当前 frozen edited-resend 原事实路径不变。

回归源码：`core/nola-application/tests/effective_model_parameters.rs` 十一项 actual-Store case，覆盖 sparse/null/零值、seed inherit/clear/value、optional clear、精确 pin/选择、candidate、本机 baseline 不可用、source Host 不可用、raw profile null 拒绝、current reasoning/protocol、实际 Application dispatch 无 receipt/Run/会话写入及敏感字段不外露；原 expert preview actual-Store suite 另增一项同快照有效参数/raw declaration 不改断言。**本批实际结果见第 7 节；没有平台参数 caller 迁移或真实模型验证结果。**


## 7. 2026-10-29 本批实际验证

统一冻结源后，主审批准单 Cargo 顺序队列，`CARGO_BUILD_JOBS=2`，无全仓/NDK/平台重建/真实模型。日志目录 `/tmp/nola-validation-20261028/`：

| 命令/日志 | 实际结果 |
| --- | --- |
| `cargo test -p nola_application --test effective_model_parameters`，`effective-parameters-query.log` | 首轮 8 passed / 3 failed；完整失败记录保留 |
| 同 suite，`effective-parameters-query-r2.log` | 10 passed / 1 failed；完整失败记录保留 |
| 同 test binary，`current_reasoning_and_protocol_override_use_one_contract_before_resolution -- --exact`，`effective-parameters-query-reasoning-r3.log` | 最后一项 1 passed / 0 failed / 10 filtered；exit 0。未声称重跑整个 suite 全绿 |
| `cargo test -p nola_application --test expert_message_transactions`，`effective-parameters-expert-transactions.log` | 27 passed / 0 failed / 0 ignored / 0 filtered；exit 0；含新增 preview 断言 |
| `cargo test -p nola_application --lib model_profile_resolution`，`effective-parameters-pure-unit.log` | 4 passed / 0 failed / 0 ignored / 396 filtered；exit 0 |
| `cargo check -p nola_mobile_bindings -p nola_application_bootstrap -p nola_service`，`effective-parameters-boundary-check.log` | 三包及传递端口边界编译成功；exit 0 |

因此本批共有 42 个不同实际 case 获得通过结果（query 11、expert 27、pure 4），不把 filtered 算通过。

首轮失败均修正了测试夹具，保留原生产严格校验和断言：显式模型选择当前会在写事务中初始化完整参数 snapshot，这是既定写语义。初版夹具因此未产生要测的 sparse，且已有初始化 temperature 覆盖遮蔽了后续 profile raw null；这不意味着 resolver 会忽略 raw null。修后通过真实 Store 写入全局 exact Host 模型 preference，让会话继承选择并逐值保留 sparse override，实际验证查询不补字段、不写 profile/settings/receipt/Run。reasoning 夹具修成当前 concise/detailed 三档偏好、Chat Completions low/medium/high 完整能力声明，仍断言 concise 实际映射 low，未放宽 parser。

源修仅新增了主审要求的 nonobject source guard：合法 JSON 但 array/null/string/number 的 `source_json` 明确返回错误，不 panic、不重建对象、不改用户数据；actual-Store 第十一项逐一覆盖。

当前结论仅覆盖 Core 查询及端口；本机权威 baseline、源 Host 查询接线、各端参数编辑/实际发送改用新查询仍未完成，不关闭 A06，也不以本批结果替代其 UI/模型验收。
