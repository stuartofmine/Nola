> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 会话数据、配置与执行所有权审计

日期：2026-10-15。范围：Core、Rust Host/Web/CLI、macOS、iOS、Android、Svelte Web。

后续实施见 [会话所有权实施契约](../实施与审计/SESSION_OWNERSHIP_IMPLEMENTATION_20261015.md)；LLM配置/模块交互见 [LLM场景契约审计](LLM_SCENARIO_CONTRACT_AUDIT_20261015.md)。以下缺陷描述保留审计时状态，不代表实施后仍未修复。

本文件为静态代码审计及整改设计，不表示实际设备/真实模型验收通过。审计未修改生产代码、未运行构建测试、未访问或修改用户会话数据。行号对应审计时工作区；工作区仍有其他代理并行修改。

主代理本轮补充的集成证据：`canonical_web_host_e2e` 5/5、`settings_web_host_e2e` 1/1 通过；`conversation_configuration_web_e2e` 2/3 通过，失败在 `cli/tests/conversation_configuration_web_e2e.rs:349`：SSE 有运行 trace，但 `/api/chat/conversations/{id}/traces?summaryOnly=1` 返回空。失败发生在后续实际模型参数断言之前，不能宣称参数端到端验收通过。`cli/src/store/event.rs:96` 的 `TraceRunsForConversation` 查询链路待定位；目前只是已实测的投影缺口，不预先断定根因。

## 结论

并非缺少所有会话隔离机制。Core 已有 conversation 配置 CAS、持久化请求账本与 checkpoint 所有权；Mac 已按会话保存内存草稿并捕获发送参数；iOS 已按会话分离执行器；Web 已有 Host 范围草稿仓库和 generation 检查。问题是这些机制没有成为所有异步操作必须遵守的统一边界，部分路径仍向“当前页面”而不是“发起操作的会话”回写。

优先修复：iOS 附件导入跨会话回写、Android Host 设置异步覆盖新选择、Android 新会话继承旧上下文显示。不要再用额外的 `isBusy` 或 UI 上隐藏错误代替所有权修正。

## 四层所有权契约

| 层 | 唯一职责/持久化 | 不允许承担的职责 |
| --- | --- | --- |
| Application defaults | 模型目录、凭据引用、默认生成配置、默认权限、设备能力；应用配置持久化 | 当前会话的实际模型、运行中参数、当前授权弹窗 |
| Conversation aggregate | 稳定 ID、模型选择、每模型参数 override、三档复杂度、权限 override、知识/资料引用、消息/上下文 checkpoint；Core 持久化并 CAS | 读取另一个页面的当前选项来补缺失字段 |
| Run immutable snapshot | 接受提交时解析 defaults + conversation override，冻结模型/协议/生成参数/复杂度、工具目录版本、权限基线、上下文引用及配置 revision；Run 持久化 | 重试/恢复时重新读取当前会话或最新全局设置改变原任务 |
| Conversation view state | 草稿、附件导入状态、阅读锚点、过程展开选择、侧栏选择；客户端按会话缓存 | 成为消息、请求计费、授权决定或任务完成状态的权威来源 |

“view-local”不是一个页面实例共享一份状态，而是 `(window, conversation)` 拥有的显示状态。纯展开状态不必跨设备同步；草稿如要求重启恢复，应有单独本地草稿仓库，不能混进模型历史。

参数解析流水线为 `GlobalDefaults → ConversationOverrides → ScenarioPolicy → FrozenRunSnapshot`。ScenarioPolicy 是模块用途的约束/填空步骤，不是第五个数据所有权层；不能静默覆盖显式会话参数。压缩热分叉复用原请求快照，不重新套用当前页面的配置。

### 推荐标识

- `ConversationKey = (authority/store identity, account identity, conversationID)`。本地用稳定数据库/工作区身份；远程用经过认证的 Host identity。URL 不是 Host 身份。
- `RunKey = (ConversationKey, runID)`；提交前用 `(ConversationKey, clientID, submissionID)`，接受后原子绑定 Run。
- `OperationKey = (ConversationKey, operationID, generation)`；附件导入、模型切换、设置保存、检索、历史分页均持有独立 operation ID。
- `PermissionKey = (RunKey, challengeID/nonce, resource/action digest)`。不能仅凭工具名称或当前选择回答授权。
- `ViewKey = (windowID, ConversationKey)`；展开项再加 `(messageID, partID)`，滚动用消息/Part 锚点与相对偏移，不只存绝对像素。
- 未提交草稿也分配稳定 `DraftConversationID`，不要把所有新会话永远塞进 `null` 槽。第一次接受提交时明确迁移草稿到 ConversationKey。

### 所有异步回写必须分两步

1. 对 owner 仓库提交：验证 owner 仍存在、operation ID 未被替代/取消；该操作允许后台完成时写回原 owner，不能因为页面已切换而丢失正确结果。
2. 对可见投影刷新：仅当当前 ViewKey 与 owner 相同，且 selection generation 仍匹配时发布。旧任务 `finally/defer` 也只能结束自己的 loading 状态，不能清掉新任务。

删除需 tombstone/revision 检查，避免迟到回调复活已删除会话。单纯 `Task.cancel()` 不代替身份验证。

## 已确认的高优先级缺陷

### P1：iOS 附件导入把结果追加到切换后的会话

- `Nola/UI/ChatView.swift:563–573`：切换会话保存/恢复 `inputText` 和 `pendingAttachments`。
- 同文件 `1417–1469`：`importChatAttachments` 捕获 parserHost，但未捕获 composerConversationID、draft owner 或 operation ID；等待文件解析后直接 `pendingAttachments.append`。错误及 `isImportingAttachments` 的结束状态同样写入页面共享状态。
- 可重现顺序：A 发起慢文件导入 → 切 B → A 完成 → B 获得 A 的附件；A 草稿里没有解析后的附件。此处与网络是否取消无关。
- 修正：在展示 picker 时绑定 owner，而不只在返回回调时读取当前会话；导入状态/notice/附件归属 `ConversationDraft`；结果写回捕获的 draft，页面只观察选中的 draft。取消与删除要使 operation 失效。

### P1：Android 旧 Host 设置请求可以改写新模型连接和能力

- `android/app/src/main/java/com/nola/ui/screen/chat/ChatViewModel.kt:1043–1084`：`setModel` 捕获 modelSelectionGeneration，但在调用 `refreshHostAgentSettings()` **之后** 才检查 generation。
- 同文件 `1215–1230`：refresh 捕获 configuredHost，await `fetchHostAgentSettings()` 后没有 owner/generation 校验，写 `webAgentProvider.config = configuredHost.copy(...)`，更新当前 pairedHostIdentity。
- 同文件 `1301–1315`：还会无条件写当前 hostUploadedSkills、hostDocuments、MCP、executionBackends 和 dataSources。
- 可重现顺序：A 的 Host 设置请求挂起 → 切 B/切本地模型 → A 返回。外层 generation 只能拦住后面的选择提交，拦不住 refresh 已发生的 provider 与 UI 修改。
- 修正：获取函数接收显式 frozen Host connection，返回纯 `HostCapabilitiesSnapshot`，不写共享 provider。通过 ConnectionKey + generation CAS 发布。身份信任记录可按原 Host 存储，但不可改变当前 Host/provider；连接运行时也应按 Host 分实例。

### P1/P2：Android 新会话保留旧会话的上下文指标

- 同文件 `934–973`：`newConversation()` 使用 `it.copy` 清消息、thinking、tools、permission 等，但未清 `contextUsage`、`contextTokens`、`contextUsageDetail`。
- 同文件 `369–378`：这些字段在加载会话时明确装入旧会话指标（包括缓存字段）。新会话没有可加载的 projection 替换它们。
- 因而新空白会话在后续真实更新前可显示前一个会话的上下文/缓存。不可修成“每次统一归零”——未知缓存不是 0% 命中。
- 修正：使用 `ConversationPresentation.empty(owner)` 构造新 owner 的完整投影；上下文和缓存用 `unknown / measured / stale / unavailable`，不能使用跨 owner 的 `copy` 延续。

## 架构缺口与产品选择（不是已证明的数据损坏）

### 草稿隔离已部分具备，但重启持久化不一致

- Mac：`NolaMac/Features/Chat/MacWorkspaceModel.swift:191–225` 的 draftsByConversation/attachmentsByConversation 是内存 map。
- iOS：`Nola/UI/ChatView.swift:84,563–573` 是视图内 map，文本和附件随页面生命周期丢失。
- Web：`web/frontend/src/lib/workspaceDrafts.ts:7–10` 明确只保存 mounted workspace 内存，不写 disk/localStorage/sessionStorage；换 Host 清除旧引用，安全隔离正确，但刷新丢草稿是既定行为。
- Android：`ConversationComposerDrafts.kt:30–47` 只将文本和 picker owner 进入 saved-instance bundle，附件/URI grant 不持久化。不能据此宣称进程重建后附件恢复。
- 建议统一承诺：切换必保留；文本草稿重启可恢复；附件持久化的是安全资源引用与导入状态，权限失效时明确提示重新选择，不能存秘密内容到普通偏好项。

### 会话切换与运行容量必须分开

- Mac `MacWorkspaceModel.swift:1520–1536` 明确工作区单运行容量；`isRunning` 或部分恢复状态阻塞另一会话发送，但 Stop 的显示已有 owner 判断。
- 这不等于串会话数据，但造成“另一个会话不结束，新会话不能用”。若单执行槽是资源约束，应表现为可取消的排队/容量状态；不能把 A 的 unknown/recovery 当成 B 的运行状态。
- iOS `ChatManager.swift:196–219` 已按 UUID 管理 execution；但根 `canBeginNewMessage` 仍受全局 unknownEffectPendingConversationID 影响。需将全局安全限制记录为具体 resource lease，而非无解释全局忙碌。
- Android `ChatViewModel.kt:276–299` 远程切换只 detach transport，保留 Host Run，方向正确；`newConversation:947` 调用统一 cancelGeneration，本地与远程执行分支的取消语义需要专门动态测试，不能仅凭注释认定两者都能后台执行。

### 模型默认与“上一个所选模型”仍需明确区分

- Android `ChatViewModel.kt:321,327,364`：缺失会话 model 时 fallback 当前 uiState.currentModel，而非独立 default selection。
- iOS `ChatView.swift:1875` 只有 conversation.selectedModelInfo 非空才恢复选择；`ChatManager.swift:1776–1805` 分离 selectModel 与 persistSelectedModel，但 selectModel 默认还更新应用默认。
- 应定义明确 inherit 语义：已有会话没选模型时解析 app default，不继承页面刚从另一会话带来的 currentModel。新建会话沿用上次模型若是产品需求，应在创建时显式写入，而不是每次打开动态推断。

### 阅读位置尚未做到“返回会话恢复阅读”

- Web `chatScroll.ts:63–70` reset 总设 latest；`Chat.svelte:434` 每次切换调用 reset。
- iOS `ChatMessageTimeline.swift:303–313` 切换总设 followsLatest 并安排跟随。
- 这能防旧异步滚动污染，但不能恢复 A 原本阅读的位置。若要求会话 UI 状态跟随会话，需 ViewKey 的 anchor/intent store；不应将滚动状态持久化到 Core 业务记录。

## 已有正确基础：保留并统一使用

| 领域 | 已有实现/证据 | 统一要求 |
| --- | --- | --- |
| 配置持久化 | `cli/src/web_parts/conversation_configuration.rs:27–51,80–106` 读取 canonical metadata/settings + expectedRevision | 客户端编辑必须带 owner + revision；返回明确 conversation identity |
| 参数合并 | `web/frontend/src/lib/chatSessionSettings.ts` sessionConfigurationPatch 按模型 key、null 清除 override；Host `native_bridge.rs:4014–4058` 读取对应 conversation overrides | override 不是直接修改 model center 默认；运行时只用 frozen resolved snapshot |
| Mac 发送冻结 | `MacWorkspaceModel.swift:2347–2418` 捕获文本、模型、复杂度、参数、权限 | 后续异步步骤不得重读 selectedConversation/current controls |
| iOS 执行隔离 | `ChatManager.swift:196–219` conversationExecutions + per-session client；`ChatView.swift:1410` Stop 指向当前 conversation execution | execution 生命周期高于 View，非当前会话继续提交到 owner store |
| Web 切换/回调 | `Chat.svelte:356–358,420–450,494–552` owner+generation，detach stream，按 owner恢复提交 | 区分 observer cancellation 与 durable Run cancellation；observer可重建 |
| 累计缓存 | `core/nola-store/src/committed_run_context_usage.rs:83–146` 按 conversation 下 Run 汇总，远程不重复计本地复制事件，attempt 去重 | 展示读投影，不按收到 SSE 次数加计；未知请求计为 unreported，不伪造 0% |
| 压缩 checkpoint | `core/nola-store/src/context_checkpoint_store.rs:8–80` 检查 component checkpoint owner，冲突更新要求相同 conversation | 压缩任务用 checkpoint base revision + Run snapshot CAS，失败不能覆盖新上下文 |
| CLI 渲染 | `cli/src/terminal_ui.rs` 单 terminal surface owns history/chrome，进度/权限分离 | 单终端 surface 不能成为 session 的业务状态；切 session 要重建投影，保存必要 draft/anchor |

累计命中率应是所有可报告请求的 read tokens 总和 / 对应 input tokens 总和，并显示未报告次数/覆盖率。不得平均各消息百分比，也不得因分页、压缩或重连清零。现有 Core 账本是正确边界，主要风险在客户端切换投影。

## 最小完整整改流程

1. 在各端建立一个 `ConversationWorkspaceState` keyed store；按字段拆分 durable aggregate、draft、presentation、observer。禁止对整页状态任意 `copy` 跨 owner。
2. 设置编辑器以 `(ConversationKey, configurationRevision)` 建 session；保存结果只更新原会话仓库。发生 revision conflict 提示重新加载差异，不整包覆盖。
3. 提交创建 SubmissionIntent，包含 idempotency key、原会话、配置 revision、模型意图、附件引用。Host 在 admission 边界解析并持久化 RunSnapshot，然后启动执行。
4. worker 不读取 selection；输出按 RunKey 提交 canonical ledger。客户端接收只是 projection reader。后台输出持续记账，但不控制前台输入框。
5. 切换只 detach observer / 切 ViewKey；恢复查询 ledger，不重新发送用户 prompt。显式 Stop 发送 RunKey + cancellation intent 并等待 terminal/unknown confirmation。
6. 授权作为 Run challenge，策略 override 属于 conversation。更改会话策略仅影响下一 Run，运行中变更需显式 Core 命令及审计事件；自动批准也不绕过 OS sandbox 或无效 schema。
7. 草稿和附件操作绑定 stable owner，清理只清已提交版本，禁止慢成功回调清掉用户后来输入的新草稿。
8. 上下文/缓存/标题/历史全部从 owner projection读取；UI 选择态与 projection subscription 分离。全局目录更新最多使下一次参数解析变更，不能改运行中 snapshot。

不需要维护旧接口兼容层。先迁移所有调用者到显式 owner 契约，再移除无 owner 的 mutation API。不能在各个回调散落关键词判断或当前页 fallback 来“补”边界。

## 验收矩阵

| 场景 | 必须断言 |
| --- | --- |
| A 慢导入 → B 编辑 → A 完成 | 仅 A 附件/notice/importing 变化；B 文本和附件保持；删除 A 后不能复活 |
| A Host设置请求慢 → 切 B/本地 → A返回 | B provider、Host identity、工具目录不变；A结果只缓存A；A失败不能弹B错误 |
| A模型/复杂度/权限 → B不同值 → A | 所有设置准确恢复；重启后同样；未设置值继承 app defaults 而非B |
| A发起后修改默认或会话参数 | 正在运行的所有后续模型请求和恢复仍用 admitted snapshot；新 Run 用新 revision |
| A运行/授权 → B | B不显示A的Stop/授权；A后台ledger增长；回A可恢复挑战；错误nonce拒绝 |
| A取消请求未确认 → B运行 | 取消只针对A；容量受限时B明确排队，不谎报B在运行 |
| A有缓存统计 → 新B | B unknown而非A数字；切A恢复累计；重放事件/分页/压缩不重复计费 |
| A压缩 + 切B + A新增历史 | compaction只提交A，coverage及base revision正确；B不变；原文可检索 |
| A读旧消息 → B → A | A恢复锚点及reading意图；B输出不滚A；late scroll callback被拒绝 |
| A保存设置 + B保存 + A旧响应 | 每个owner正确CAS；过期revision不覆盖，旧finally不清新请求busy |
| 页面销毁/进程恢复 | 从提交账本恢复，不重复提交；明确草稿附件恢复限制；凭据不进入会话序列化 |
| 两窗口同会话/两个Host同ID | CAS冲突可见；Host隔离，无草稿、授权、统计串线 |

测试层次：纯 reducer owner矩阵 → 持久化/真实HTTP admission与恢复集成 → 延迟夹具下的切换交互 → 真实模型任务。不能只用截图或仅验证页面出现；每步同时检查 owner存储、副作用、可见投影。真实链路出现的新失败作为回归进入同一矩阵。
