> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# iOS Run 模型配置恢复：现状与待实施边界

日期：2026-09-28。本文记录代码审查证据与下一切口，**持久恢复修复尚未实施**；不定义第二套 iOS generation DTO。

## 已完成与未完成

新发送的 `ChatManager.runModelClient(for:parameters:)` 已为 local/direct-cloud 建立独立 provider。local 路径冻结会话合并后的参数，只有模型权重服务共享；`IOSLocalRunConfigurationTests` 的 3 项模拟器测试已通过。该结果不覆盖进程死亡后的权限恢复，也不证明真实模型执行。

进程死亡后的 `restorePendingLocalPermissionWaitIfNeeded()` 仍使用当前 `container.llmClient`，并从当前 conversation/selectedModel 和 `currentContextTokenLimit()` 构造配置，输出上限固定为 16,384。不能把新发送的隔离修复宣称为恢复修复。

## 已证实的断层

1. `Nola/Services/ChatManager.swift` 的 `restorePendingLocalPermissionWaitIfNeeded()` 创建 `TextAgentModelClient(client: container.llmClient)`，没有通过冻结 Run 的 provider factory。`generationParameters(for:conversationID:)` 读取当前全局参数并应用当前会话覆盖；`currentContextTokenLimit()` 间接使用该结果。恢复也重新载入当前已启用 Skills。
2. `Nola/Services/DeviceAgentCoordinator.swift` 的恢复入口把调用方配置保存为 `activeConfig`；`runModel` 在请求 settings 缺少 context/prompt limit 时使用该配置兜底。初始 `makeRuntimeConfig` 保存 model、context、tools、permission/catalog 和能力/提示身份，但没有完整可重建 provider 的 route 与生成参数。
3. `Nola/Services/IOSCanonicalAgentApplication.swift` 的 `recoverPendingPermissionWait()` 正确检查 exact Run/assistant、原 invocation digest，并只领取已提交的 permission delivery。共享 Application 的 `load_run_invocation_config` 校验原配置内容摘要，后续 Core reduction 仍使用原配置。**Core 冻结正确不等于外层 provider 冻结正确**。`ActiveRun` 查询只返回摘要，不提供可重建 provider 的完整快照。

## Swift provider 实际消费字段

| 路径 | 真实需要的字段 |
| --- | --- |
| direct-cloud | 完整 base URL、credential、effective upstream model ID、wire protocol（缺省 chat-completions）、execution capabilities、temperature、top-p、输出上限、presence/frequency penalty、seed、stop 数组 |
| local | 精确本地模型目录身份、temperature、top-p、top-k、输出上限、repetition/presence/frequency penalty、seed、thinking 开关、max-context（实际传入 MLX max-KV-size） |
| Core/协调层 | 原 context/output 配置、原能力/提示身份、工具与权限策略绑定 |

local 不支持非空 stop；已有 factory 明确拒绝。cloud 当前 factory 不传 top-k/repetition，不能在恢复文档或 DTO 中声称它们已执行。max-KV-size 不是模型原生上下文能力扩容。

`ModelParameters` 还含展示/提示字段，不能不加区分地把整个 UI 结构当成新的共享 generation 合同。

## Credential、profile 与 endpoint

- `ModelSelectionManager.saveCloudAPIKey` 使用 Keychain service `nola.cloud-model`、account 为 profile/model ID；对应 reader 当前为 private。
- `resolvedCloudModelForRun` 允许在 route 匹配时使用 selected model 携带的临时 API key。恢复必须使用明确 credential reference 并验证绑定，**不得 fallback 到当前 selected model、当前默认 profile 或临时 key**；缺失/变更应明确不可恢复。
- `IOSCanonicalModelProfile` 有 ID/revision，但现有查询读取当前记录，没有按旧 revision 读取历史 profile 的接口，`ModelInfo` 本身也不携带 revision。因此仅存 ID/revision 无法重建旧 route。必须保存已验证的可重建非 secret 字段，或对当前来源做 exact 比对并在不一致时拒绝；不得用当前值替换。
- `OpenAIProvider.chatCompletionsURL`/`responsesURL` 只改 URL path，会保留 query/user-info。现有 `ModelInfo.redactedForPersistence` 仅剔除 API key，并不保证 baseURL 没有 secret。新快照不能复制整个 ModelInfo，也不能把完整 URL 无条件持久化。

## 下一最小完整修复方向（未实施）

沿共享 `HostInvocationBinding` 扩展可重建字段与 exact 读取边界，复用 Domain `GenerationOptions`、`ReasoningSelection` 等合同。现 `HostGenerationBinding` 不含 top-k/penalty/seed/stop，不能假定已有 binding 足够完整。先统一共享 schema，再增加 Swift 严格镜像及适配；不要先造 iOS 独立持久 DTO。

新发送先冻结真实执行参数/route，再与 Run 创建**原子持久化**。恢复在 claim delivery 或执行模型之前，只读加载并验证 exact conversation/Run/assistant、版本/摘要、route、credential reference。按快照构造独立 provider，不读取当前 UI/defaults 重建参数。旧 Run 缺少有效快照时明确保持不可恢复，不猜值、不重发原 prompt。

权限实时撤销必须获胜：配置冻结不是权限授权快照的永久放行依据。应保留原策略约束并重新验证当前资源授权，不能把恢复时当前更宽松设置当作原授权。

Mac 可复用的是“冻结请求 + exact identity + 重新校验授权”的原则。`MacLocalAgentOwnedLaunchSnapshot` 明确仅是内存 owner，不足以支持 iOS 进程死亡恢复；其 argv/environment 也不能作为持久 iOS 模型快照直接复制。

## 共享可重建合同：generation 切片已实施，其余待实施

本节的完整恢复方案仍未实施。已落地的有界切片是 binding v3 显式保存
`reasoning_selection`、`sampling_override`、`generation_options`；三字段必填，
旧 v2 不可作为有效恢复绑定。Domain `ValidatedGenerationOptions` 私有封装
经 `GenerationOptions` 校验并重新编码的规范 JSON 值，提供 Eq 和严格反序列化，
不复制 options schema。CLI 已将三项移出 policy hash 并保存真实值。
随后同批 binding 升至 v5（output admission v3），显式绑定实际 wire protocol
与已解析的 output-token 字段，并增加 required `max_retries` 和
`parallel_tool_calls`。CLI 新 Run 准入只解析一次 retry 环境配置；恢复从原
binding 回填，不重新采用当前环境。没有准入 retry 值时，binding 构造拒绝。
`parallel_tool_calls=None` 保持请求字段省略；step 能力只能把已有 `Some(true)`
收紧成 false，不能凭 capability 新造偏好。收到超大工具批次后的显式纠错
`Some(false)` 是独立运行时安全处理，不覆盖初始冻结偏好。
route credential/ref、cache 原值重建及 iOS 恢复 provider 接线仍未实施，
**这些版本均不表示已经可以完整冷恢复**。
当前 tagged route 切片进一步升级为 Invocation v6 / OutputAdmission v4。
`kind:http` 保留 v5 HTTP 字段；`kind:native` 使用 `adapter:mlx`、
`adapter_contract_revision:1`、逻辑 `model_id` 及 required 可空 `profile_ref`
（非空时为 profile_id / admitted_revision）。native 明确拒绝 HTTP 字段；
两个 admission/binding 校验入口都验证 route。它证明逻辑模型选择，不证明
权重内容或文件不变；HTTP credential 仍只有摘要，本批没有实现秘密引用恢复。
以下其余字段是后续修订建议（breaking 修改须再次递增版本）。只扩展现有
`HostInvocationBinding`、`HostGenerationBinding` 和 `HostModelRouteBinding`，
不增加并行 Run snapshot 或平台独立 generation DTO。

### 摘要与可重建数据的区别

本切片之前，CLI `core_agent_host_invocation.rs` 把 `reasoning_selection`、
`sampling_override`、`generation_options` 放入 `execution_policy_sha256` 的
输入，未保存这些值本身。摘要可发现配置变化，却不能恢复原参数。
当时 `chat_output_token_field` 未进入绑定，Host 重建时 `LlmConfig.max_retries`
会重新读取当前环境；不是同一个已建立 LlmConfig 每次 stream 都读环境。
上述 v5 切片修复这两个边界，不能把它扩大表述为所有 provider 恢复已完成。

现有 credential、endpoint query、prompt-cache key/prefix 的 SHA 也只有比较
用途。特别是 cache key 摘要不是可发送的原始 cache key；重试次数是 transport
策略，不是 generation 参数。禁止把这两类字段混称为已持久化的可重建生成配置。

### 最小字段与复用边界

- `generation` 增加最终 resolved `Option<ReasoningSelection>`、
  `sampling_override`、完整 `GenerationOptions`；复用
  `nola_domain::generation`，不从三档 UI 标签重新计算 effort/budget。
  保留已有 temperature、top-p、已准入 output、thinking 与缓存策略字段。
  `None`/空 options 是明确最终值，不得恢复时再次继承当前默认；显式 seed=0、
  penalty=0、stop=[] 必须保留。
- 每个 primary/fallback route 使用显式 Domain `ModelProtocol` 和
  `ChatOutputTokenField`。需要能力判断的适配器复用 Domain
  `ModelExecutionCapabilities` 的已验证声明，不从 model 名称推断。
  保留精确 model、规范化非秘密 endpoint 与 fallback 执行顺序；不能仅靠
  provider 字符串恢复协议。
- 非秘密 route 来源至少区分 Host profile、Host configuration 和 local
  runtime。profile 路径保存 profile ID、准入 revision、endpoint ID 及
  credential account reference；local 路径保存明确模型/artifact/launch
  身份。它们用于精确定位、证明与权限复验，不替代冻结的 route/生成值，
  也不要求普通 CLI 临时配置伪造一个 profile。
- 另在既有 binding 内显式冻结 transport retry 策略及
  `parallel_tool_calls` 请求偏好；缓存策略区分关闭、可重建非秘密配置与
  必须通过受限 reference 获取原值的模式。不得把 retry 或 cache 摘要
  填进 `GenerationOptions`。

### Eq、版本与校验

`ModelProtocol`、`ReasoningSelection`、`ChatOutputTokenField`、
`ModelExecutionCapabilities` 已支持 Eq。`GenerationOptions` 包含 f64，当前
只支持 PartialEq；直接嵌入会影响 Host binding 和 `AgentTurnIdentity` 的
Eq 派生。实施时优先在共享数值层建立经验证的有限值表示（可沿现有
`serde_json::Number` 方案），继续复用同一个 `GenerationOptions`，不要复制
一套 Host options，也不要为可构造 NaN 的原始 f64 强行实现 Eq。
数值规范化与往返必须有测试，不能悄然改写合法值或抹掉 Option 的存在性。

已实施的 generation 切片将 binding 版本由 2 升至 3，新字段必填且拒绝未知字段；旧版缺失
不能用 serde 默认解释成关闭 reasoning。完整恢复必须先校验版本和内容摘要。
已提升为 typed 字段的生成配置应退出 execution-policy 摘要的职责范围；
该摘要继续约束工具/工作区等 Host 策略，但仍不构成可重新授予权限的凭据。

### 不可重建及秘密边界

profile ID/revision 不是历史配置数据库：当前没有旧 revision 读取能力时，
必须使用 binding 内冻结的非秘密执行值。profile 删除、来源不符或凭据不可用
必须拒绝恢复；不能用当前同名模型、selected profile 或新 endpoint 替代。

临时 API key 没有 durable credential reference 时只能声明不可冷恢复。
含 query 的 endpoint 必须通过明确、受限的 resolver 找回原 query 并比对摘要；
仅凭规范化 endpoint 和 query SHA 无法重建。找不到 resolver 时拒绝，而不是
丢弃 query 继续请求。同样适用于不可重建的 cache key/prefix。
不得落盘 API key、授权 header、原始 secret query、argv/environment 或整个
profile `source_json`。凭据引用解析成功后仍须验证冻结身份；本提案不隐式
允许 credential rotation 改变原 binding。

共享验收应同时证明：全局/profile 默认变更不改变冻结请求；三种协议、字段
省略与显式零、fallback 顺序可重建；v2/缺引用/错误秘密摘要拒绝且无执行；
持久 JSON 与错误日志不泄露原始秘密。完整 provider 恢复仍需下节真实 DB
测试，不能用 binding 序列化往返代替。

## 验收切片

- 真实隔离 SQLite：原 Run 等待权限，重建应用对象，修改全局默认、会话参数与当前模型；恢复后注入的真实 provider 执行闭包仍观察到原配置，Run/消息身份不变。
- 缺快照、版本/摘要不符、错误 assistant、旧 profile 无法重建、credential 缺失或被替换：恢复明确拒绝，不 claim、不调用模型、不创建新消息。
- 授权撤销后恢复不能执行原工具；新增授权不能扩大冻结策略。重复恢复不得重复领取有效 delivery。
- GenerationOptions 映射覆盖缺省与显式零：local output=64、context=4096、temperature=0、top-p=0、top-k=1、repetition=1.2、presence=-0.5、frequency=0.75、seed=0、stop=[]；cloud output=64000、temperature=0、top-p=1、presence/frequency=0、seed=0、stop=["END", "终止"]。local 非空 stop 拒绝；cloud stop 沿当前换行分割/trim/nonempty 规则转换。
- `IOSCanonicalAgentApplicationContractTests.testColdPermissionRecoveryClaimsFreshFencedDeliveryAndRestoresDigest` 现有测试仅证明 digest/fencing，不足以替代上述 provider 恢复与真实持久化测试。

截至本文更新，共享 Rust generation 绑定切片已实施；未新增独立恢复 DTO，
未完成 iOS provider 冷恢复接线，也未运行恢复模型网络验收。

## 已接入的共享精确读取（不等于恢复完成）

后续已接入 `LoadExactRunInvocation` 只读 Application 命令。Store 在同一 pinned
snapshot 中检查精确 conversation/Run/assistant、本地所有权、消息归属与配置存在性。
无 Run 返回 null；存在但身份不符、远端归属或配置缺失明确报错。Application 验证
原始配置字节的 SHA-256、解码 Core config、校验既有 HostInvocationBinding，只返回
身份、状态、摘要和绑定，不返回原始 prompt/tool/context JSON。

28151真实隔离SQLite Application回归2/2通过：冷重开读取、显式零值、只读命令分类、
缺失Run、身份不符、损坏摘要、缺binding和含敏感query的地址拒绝；验证原始工具数据
不会出现在结果中。此项读取测试不表示绑定已足够重建provider，也不授权
claim delivery或恢复终态Run；使用方仍须执行版本、可重建性和实时权限检查。

Swift已接入严格的精确读取envelope及claim前准备gate：检查三个身份、64位小写hex
摘要和版本UInt32形状，binding保持opaque；prepareRecovery缺失或失败时不claim。
准备成功后重新读取并比较原绑定，再进入既有permission fencing流程。当前生产
未提供完整provider准备器，故冷恢复明确拒绝。新增4项测试与原7项fencing测试正在
66141实际模拟器专项已exit0：11/11通过、0失败/0跳过，实际Swift与FFI编译完成。
结果为`/tmp/nola-ios-exact-run-20260928.xcresult`；未调用真实模型，也未接完整provider
重建。该次编译发生在后续v4路由协议字段改动之前，不作v4 FFI验收证据。

后续42117回归增至3/3：有Host binding的提交在快照序列化前验证绑定，敏感URL
不会先落库再等待恢复校验。Store18746全195/195通过，其中exact读取3项验证
真实WAL固定快照、冷重启及身份/remote拒绝。这些证据仍不覆盖Swift恢复执行。

### 后续重建的整数保真限制

重新核对当前实现后确认：`AgentCoreJSONValue` 不只有 Double `number`，已有
`signedInteger(Int64)` 与 `unsignedInteger(UInt64)`。decoder 对大于安全整数范围的
数值先尝试精确整数解码，encoder 也保留整数。因此不能把此前仅观察 number case
得到的风险推断写成“现实现已丢失 seed”的事实。

`IOSApplicationRuntime` 的实际路径是 Rust C JSON 字符串 → Foundation JSONSerialization
receipt → JSONSerialization result bytes → JSONDecoder/AgentCoreJSONValue。独立 Foundation
运行验证已确认 2^53+1、Int64.min/max、UInt64.max 经该中间序列化步骤未改值。
新增 `IOSExactRunInvocationTests.testRawReceiptIntegersRemainExactThroughFoundationAndOpaqueBinding`
复现同一路径并通过实际 exact envelope 校验、再编码比对；当前该新增测试仅 parse 通过，
待统一模拟器验证。UInt64.max 样本只验证通用 opaque 容器，不代表它是合法 i64 seed。
无需新建整数容器或全局 JSON enum 改动；未来 provider 重建仍须消费精确整数 case，
不能显式转换到 Double 后再恢复 Int。当前仍无完整冷恢复 provider 执行能力。
