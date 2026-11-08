> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 移动端模型身份与原子创建审计

日期：2026-10-15。性质：实际源码只读审计及接口设计；本文不是全端迁移完成声明。

进度说明：下文第 1–5 节保留实施前审计快照，不能当作当前待办清单。移动端身份迁移已实施，iOS 74 项通过、Android 431 项通过及 2 项跳过；共享 Core/CLI 本批 62 项通过。Mac 配置写入与异步提交门禁修复后 43 项通过。最新完成边界与未完成事项以 [统一契约](../../软件架构设计/模块/04-存储与索引/CONVERSATION_AGGREGATE_CONTRACT_20261015.md) 及各端模型身份实施文档为准。

上位契约：[会话聚合](../../软件架构设计/模块/04-存储与索引/CONVERSATION_AGGREGATE_CONTRACT_20261015.md)、[LLM 场景契约](LLM_SCENARIO_CONTRACT_AUDIT_20261015.md)。代码定位为审计时位置，其他代理正在实施，后续行号可能变化。

## 1. 已确认的问题与进度边界

1. iOS `ChatManager.startConversation`（`Nola/Services/ChatManager.swift:1728`）依次调用 `createConversation`、`upsertConversation`、`saveConversationParameters`。后两步失败时第一步已提交，可能留下缺少模型或参数的会话。`LocalConversationStore.createConversation`（`ConversationStore.swift:236`）原接口只接收标题、语言，无法表达完整初始化输入。
2. 审计时 iOS、Android 未读写 `modelSelectionID`，加载身份仍来自 `selectedModelInfo`。共享 Host/CLI/Web 修改 canonical 选择后，移动端可能继续使用旧身份；不能再增加一个独立可写的 `modelSelectionID` 而保留原双写规则。
3. iOS `ModelSelectionManager.canonicalProfile`（`:640`）明确设置 `profile.id = model.id`，wire 模型名为 `effectiveUpstreamModelId`。Android `domain/model/Models.kt:105` 明确 `ModelInfo.id` 是 wire 模型名，`profileId` 才是 canonical identity。不能跨语言照搬字段名称。
4. Android `ConversationParameters.kt::conversationParameterKey` 使用 `parameterProfileId`（`profileId ?: id`），但 `ConversationGenerationOverride.modelKeyOrNull` 对 Cloud 缺少 profileId 返回空。二者对缺失身份的处理不同，可能把 wire 名称写成 profile 主键。
5. iOS `ModelSelectionManager.modelInfo(from:)`（`:595`）要求 profile.sourceJSON 可解码完整 iOS ModelInfo，并要求 id 相等。Host 标准模型 source 并不保证包含 iOS 展示字段，不能因此回退旧 `selectedModelInfo` 或当前全局选择。
6. Android `ConversationRepository.createConversation`（`:89`）已先合并参数到 settings JSON，再单次创建，原子结构正确；`ChatViewModel` 模型选择（`:1071`）仍先 `saveModelParameters` 再 `updateModelInfo`，并非同一事务。
7. 两端 runtime 创建包装原先忽略 `created:false`。主代理已告知本批修改为拒绝该回执并补回归；本文未重新执行测试。iOS 代理正在收口创建和模型选择事务，不能把审计时问题写成已验证修复。

## 2. 单一身份规则

`settings_json.modelSelectionID` 是会话模型选择的唯一可写事实。`selectedModelInfo` 应变为按身份加载模型目录得到的显示/调用准备投影，不再作为第二份持久身份。参数使用同一稳定 key 的 `modelOverrides[key]`，不能用显示名或 wire 模型名索引。

| 场景 | canonical identity | 映射与约束 |
| --- | --- | --- |
| 当前 Store 的远端模型 | `configured:<profileID>` | iOS 当前直连 profile 使用 `model.id`；Android 必须使用非空 `profileId`。wire 模型名仅作为执行字段 |
| 当前设备本地模型 | `local:<localModelID>` | 按本地目录解析能力与可用性；同名并不代表跨设备已安装，缺失明确提示 |
| 连接 Host 的具体模型 | `connectedHost:<encodedHostID>:<encodedProfileID>` | 沿用共享校验器的 URL-safe Base64 无 padding 编码；Host scope 必须参与身份，不能只保存远端 profileID |
| WebHost 连接入口 | 不是一个模型身份 | 表达 execution-host binding；只有选定该 Host 的具体模型后才能形成 connectedHost identity。不得写 `configured:webhost` 或固定 `local-host` 虚拟 ID |
| 尚未选模型的草稿 | 草稿态显式未选择 | 不等于已创建会话永久继承全局默认。提交创建时明确解析默认一次，或返回配置不足 |

共享 identity 校验位于 `core/nola-application/src/conversation_configuration.rs::valid_model_identity`。允许某种 identity 格式不代表每个客户端已实现对应执行适配；尤其移动 WebHost 入口与 connectedHost 模型选择不能混为一谈。

目录解析失败时返回 `unavailable`/`missing profile` 状态，保留会话记录；不要选另一个模型继续。凭据只能按目录中的安全引用绑定，不进入会话展示快照或草稿。

## 3. 模块输入输出与所有权

| 模块 | 输入 | 输出 | 禁止行为 |
| --- | --- | --- | --- |
| 草稿仓储 | Host/store scope、draft ID、文本/安全附件引用、显式草稿设置 | 可恢复草稿及 promote 结果 | 把 nil 会话作为所有新草稿的共同 owner |
| 创建配置解析 | 显式选择或创建时应用默认、模型目录、参数初值、权限及场景设置 | 完整 settings JSON 与固定新 conversation ID | 创建空壳后逐次补模型和参数 |
| canonical 创建 | `CreateConversation{id,title,settings_json,timestamps}` | `created` 回执；提交后的 owner/revision | 忽略 `created:false` 并当成新建成功 |
| 模型选择命令 | conversation key、expectedRevision、modelSelectionID、必要显式参数 | 新配置 revision 与 canonical 投影 | 先改 UI/provider，再分别写参数和身份 |
| 模型目录适配 | modelSelectionID、Host scope、canonical profile | 无密钥展示对象、能力描述、执行准备引用 | 依赖另一个平台专属 sourceJSON 才能读身份 |
| 会话加载 | scoped conversation key | 已保存选择、参数、消息/Run/统计投影 | 从上一会话或当前全局默认填充缺失模型 |
| Run 准入 | canonical owner/revision、具体执行输入 | 冻结调用配置、运行身份 | 后台读取当前窗口选择或漂移参数 |

缓存统计、上下文、消息、Run、Plan/Goal 继续由既有 canonical Store 持有，不为移动端再建账本。

## 4. 原子 API 的最小正确修改

### 新建

不需要新增 Core 创建协议。已有 `core/nola-store/src/conversation_profile.rs::create_conversation` 在同一事务执行：检查 ID、初始化选定模型参数、插入 conversation、插入 settings、发布内容变更、提交。Application `conversation.rs::create_conversation` 在写入前验证 settings。

客户端仓储应接收一个完整初始化值，而非仅标题/语言。先在内存完成模型身份、参数、权限和展示字段编码，再调用一次 `createConversation(settingsJSON)`。成功回执之前不设置 currentConversationId、不清草稿、不发送远端请求；失败保留原草稿。成功后以回执/读回的 canonical owner 为准发布身份，然后 promote 草稿。promote 若失败应报告可恢复草稿状态，不能重新创建另一个会话掩盖失败。

### 切换模型

沿用 `PatchConversationConfiguration{conversation_id,expected_revision,patch_json,updated_at_ms}`，在一个 patch 中提交 `modelSelectionID` 和显式 `modelOverrides`。Store `initialize_selected_model` 在同事务补齐首次选用模型的默认参数，已有会话参数保持。客户端无需复制一套默认策略。

提交成功后加载返回配置并更新显示模型；失败保持原有效选择，并显示冲突/不可用原因。保存参数和保存 selectedModelInfo 两个命令不是原子模型切换的替代品。共享配置 patch 负责模型字段；展示偏好可用已有 metadata CAS，但不允许 metadata 维护另一份模型身份。若展示项必须和选择共同成功，需明确扩展同一个配置命令的展示字段契约，不能默认多个 CAS 等价于一个事务。

### 模型目录读取

iOS/Android 从 canonical profile 的 id、displayName、modelName、runtime、source 中标准协议/能力字段构造 ModelInfo 投影；端侧独有展示字段可以缺省。要区分“展示缺字段”和“模型身份不存在”，不能因解码失败构造另一个随机身份。

## 5. 尚未完成或未验证的边界

- 移动端全面迁移为 modelSelectionID 单一身份的代码与调用点验收；本审计仅提供证据与建议。
- 两端 WebHost 连接对象到具体 connectedHost 模型身份、远端目录 revision 的完整映射。
- local 模型跨设备不可用时的呈现与恢复，不承诺只存同一 key 就可运行。
- UI 模型选择先写 provider 再持久化等调用点，需要实施代理逐一收口；不能只修仓储而保留乐观执行切换。
- 创建回执、配置 CAS 冲突、草稿 promote 失败的真实交互验收。
- configured 模型缺失时，完整已有参数不会等于具备可用 provider/凭据；必须在准入单独校验。

## 6. 验收矩阵

### 共享 Core 本批实施边界

配置命令现支持无凭据 `executionHostBinding={baseURL,tlsCertificateSHA256?}`、canonical generation 格式 `clientHostParameters`，以及 `clientModelPresentation[execution-host]` 的共同显示字段。Host 绑定与非空 modelSelectionID 互斥，切换由一个 CAS 同时清空旧身份并设置新身份；合并后再次验证互斥，不能靠省略字段绕过。URL 仅 http(s)，禁止 userinfo、query、fragment，证书 pin 为 64 位十六进制。Host 参数不接受各客户端原生序列化字段名或明文密钥。

显式创建/模型选择必须查询启用的 profile；完整 modelOverrides 不再绕过可用性检查。已有会话仅保存其他字段且 modelSelectionID 不变时不重新解析 profile，故模型后来删除/禁用不会阻止浏览或编辑标题；提交仍由运行准入判断可执行性。metadata 禁止改模型身份及 Host 绑定，但保留合法共享参数和展示字段写入。

具体模型选择命令在未同时显式提供非空 Host 时，同事务清除原 Host binding 与 clientHostParameters，作为单一执行目标领域规则，确保 Web/Mac/CLI 可接管移动端 Host 会话；同时显式提供两个非空目标仍拒绝。移动端仍发送明确的清除字段，降低客户端歧义。

#### 本批统一 Rust 验证（2026-10-15）

全部实现后统一验证，62 项通过：`cargo test -p nola_application --lib conversation` 42 项；`cargo test -p nola_store --lib configuration_initialization_tests` 2 项；`cargo test -p nola_cli --lib conversation_invocation::tests` 4 项；`--lib session_override_translation_tests` 5 项；`--lib store::web_conversation::tests` 6 项；`--test conversation_configuration_web_e2e` 3 项。均单测试线程，CLI 测试设置 `RUST_MIN_STACK=16777216`。`cargo check -p nola_cli`、`git diff --check` 通过。首次 Web 仓储测试发现旧 fixture 选择未注册 profile；补合法测试目录后通过，未放松生产可用性规则。CLI 测试链接仍有 Apple ld `__eh_frame >16MB` 警告。此记录不代表移动 UI 真机验收或远端真实模型验收。

唯一选择命令为 `patch_conversation_configuration`，新增 `clientModelPresentation` 参数允许身份、生成参数和展示参数同一次 CAS。该字段按 model key/字段合并；null 删除字段或整个模型展示对象，不覆盖其他模型。允许 showThinking、streamOutput、enableRetrieval 布尔值，以及 responseStyle、customSystemPrompt 有界字符串；它们不授予工具或数据权限。

metadata 命令禁止写/删除 `modelSelectionID`，避免绕过选择校验和初始化；现有 metadata 写普通字段及合法 modelOverrides/presentation 的调用仍可用，避免本批误伤非模型选择操作。完整 settings create/update 复用身份与展示字段校验。Store 在合并显式模型参数后才初始化缺失默认值，保证一个命令可携带完整初值，不要求先创建参数再写身份。本批不重构所有整包 settings API，也不宣称所有模型目录 source 已标准化。

| 用例 | 必须观察到的结果 |
| --- | --- |
| A/B 不同模型、参数、权限，快速切换 | 只读取目标 owner；A 输出不改变 B 输入配置 |
| 创建过程中 settings 编码或 Store 写入失败 | 没有半初始化 canonical 会话，草稿仍在 |
| 重复 ID 返回 created:false | 不发布为新会话，不迁移/清除草稿 |
| 创建后修改全局默认 | 已有会话选择和参数不变 |
| 同一次模型选择初始化参数失败 | 模型身份和参数都不变 |
| 两客户端同时编辑同一会话 | 陈旧 expectedRevision 被拒绝，不用新读 revision 掩盖旧编辑基线 |
| Host 改模型后移动端加载 | 由 modelSelectionID 解析新选择；不读旧 selectedModelInfo |
| Android 相同 wire 名称、不同 profileID | 参数与选择不相互串用 |
| 同 profileID 位于两个 Host | scope 不同，身份、参数、凭据引用不混淆 |
| 缺失 profile、关闭 endpoint、凭据不可读 | 明确不可用，不回退当前全局或其他会话模型 |
| 应用重启、后台 Run、草稿恢复 | 会话选择从 canonical 读取；已运行请求使用原冻结配置 |
| WebHost 尚未选具体远端模型 | 明确连接/选择状态，不伪装 configured 模型 |

本轮文档操作未执行编译或测试；此前 Rust 配置验证记录见 LLM 场景契约，不将其等同于以上移动端 UI 验收。
