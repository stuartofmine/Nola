> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Web 聊天 Svelte 替换验收合同

审计日期：2026-09-22。本文是完成门槛，不是已完成功能声明。依据当前工作树的 Mac 聊天实现、旧 Web 模板和 Rust Web 路由；迁移期间实现可能继续变化，验收记录必须注明对应版本。

## 2026-09-23 进展（仍未达到替换门槛）

- `/chat-next` 已作为 Svelte 预览接入真实 Host shell；`/chat` 仍由旧 HTML 提供完整功能。预览使用共享 SSE 解码、Core Part 快照、canonical 消息投影和严格提交回执；EOF、工具进度、Trace 终态不自动释放输入。
- 预览已接会话 CRUD/导出/分支/截断组件、主时间线、历史分页、草稿、IME 防误发、停止/恢复、权限面板及七语言文案。权限审批仅使用 Host 明确提供的 `safe_review_text`；空摘要禁用允许，绝不回退展示原始参数。审批提交走同源 CSRF 边界。
- 会话级模型选择、权限、稀疏模型参数和 reasoning 设置已接入侧栏。配置用 revision CAS，冲突保留草稿；显式 Web 模型选择与 native `modelSelectionID` 现于同一事务更新，真实 Host 测试验证了下一次模型请求，改标题不会重放旧模型选择。
- 会话资源已有 PDF 上传、列表、读取和移除的 Host 合同及真实 Host 测试；预览 UI 使用同源读取后临时 Blob URL，绕开 Host 对直接 iframe 的拒绝，切会话释放 URL。它明确只是预览，不是消息附件或模型提取；浏览器内实际 PDF 呈现仍未验收。
- Svelte 预览已接入旧页已有的 UTF-8 文本附件：读取中锁定发送，按会话隔离草稿，发送时冻结文件快照，只有 canonical 提交确认才清理已发送文件；真实 Host 模型请求验证了文本内容到达 provider。该合同仍是模型文本上下文，不等于 Mac 的通用文件提取或 PDF 模型附件。
- 侧栏增加常显概览，上次真实运行占用与当前准备估算由 Host 的 `usageSource` 显式区分；没有已记录用量时不显示伪造的 0%。Web 无 Mac Finder 操作，不能把会话沙盒路径控制伪装为浏览器能力。
- 对话区域改为会话导航、主时间线、详情分栏；详情按 Mac 的概览/知识/工具语义切换。工具页读取 canonical Goal 与 Host `summaryOnly=1` 的 Trace 摘要、执行真实控制回执；摘要请求不传原始 Trace 事件，避免把可能含敏感参数的数据送入浏览器组件。Browser 能力尚未接 Web，不显示伪可用标签。
- 权限模式作为输入框下方的小组件接入唯一的会话配置 owner；概览与输入区共用 draft/revision，权限单独保存仅提交 permissionProfile 与 expectedRevision，切换详情标签不会卸载 owner。七语言翻译目录覆盖 Svelte 新聊天组件；PDF/文本附件按钮和大小数字跟随应用语言。预览侧栏会话链接留在 `/chat-next`，旧页 `/chat` 仍独立保留。旧聊天模板的 552 个唯一 `tr` 文案键在德/西/法/日/俄目录有完整覆盖测试，但动态消息与真实浏览器视觉仍需验收。
- 知识页新增会话数据源控制：读取 Host `conversationOverride` 与 Core `resolution`，只提交当前源的显式启用/关闭/继承；全局 `blocked` 不显示可编辑控件，也不把源已启用误称为可读取。真实 Host 验证了 opt-in→ready、恢复继承、全局 blocked 不可读取及不存在会话返回 404。数据源授权与源内容提取仍由 Host/Core 决定，前端选项不是目录权限。
- Store 的会话数据源写入现于同一事务校验会话和源仍存在；本地/远端会话删除原子清理覆盖记录。远端完整会话投影的先删后建仍保留本机覆盖选择，不使用会误删本地选择的会话外键级联。Store 数据源与远端同步回归测试覆盖这个生命周期。
- Host 历史分页现支持 `expectedRevision`/`expectedEventSequence`；非法游标为 400，快照变化为 409，预览遇 409 重新读取首屏，避免拼接不同快照。断流后可通过原 run 恢复，或由新鲜 canonical 历史的精确 user/assistant ID 与终态验证遗漏的提交回执。
- 前端类型检查、197 项测试（含七语言实际 Svelte SSR）与构建、真实 Host `/chat-next` GET/HEAD/分页/PDF/模型选择/文本附件/会话数据源合同通过。应用内浏览器发现结果为空，视觉/窄屏/IME 的真实浏览器验收未完成。通用文件模型附件、工作上下文详情、Browser 功能及旧页全部操作的功能对等仍未验证，不得将 `/chat-next` 切为默认 `/chat`。

## 1. 边界与原则

Svelte 替换的是客户端展示、交互和状态消费，不另建聊天运行时、权限裁决器或会话事实库。Core 的会话投影、事件、工具请求及执行回执是事实来源。模型依据工具 schema 选择具体调用，Core 根据资源和权限裁决；不得用有限的中文/英文关键词场景决定路由、授予权限或强制工具。

必须区分三类承诺：

- **旧 Web 已有能力**：保留真实能力和恢复路径，不要求复制旧 DOM 或错误交互。
- **Mac 对齐目标**：只有 Web Host 已接通并通过端到端合同测试后才能显示为可用；缺 API 时明确记录缺口。
- **旧 UI 的误导性行为**：需要移除或纠正，不能以“兼容”为由继续呈现假成功、假进度或错误配置作用域。

页面能加载、SSR 能渲染、一次普通对话成功，都不等于完整替换。默认路由切换与“完整替换”声明必须分别记录；未达到下列完整门槛时，应保留明确的旧页入口或声明受限预览，不能静默移除能力。

## 2. 源码依据

| 依据 | 审计内容 |
| --- | --- |
| `NolaMac/Features/Chat/MacComposerFeatureContract.swift` | 不可变显示快照、按 conversationID 分发输入动作、附件提取状态、发送/停止、模型 reasoning 与权限模式 |
| `NolaMac/Features/Chat/MacComposerView.swift` | 原生输入生命周期、粘贴/拖放、附件托盘、输入框下方权限与 thinking effort 组件 |
| `NolaMac/Features/Inspector/MacChatInspectorView.swift` | 概览、知识、浏览器、活动分区 |
| `NolaMac/Features/Inspector/MacInspectorOverviewHeader.swift` | 真实运行态、模型/执行主机、工作区及会话沙盒显示 |
| `NolaMac/Features/Inspector/MacInspectorContextSummaryView.swift` | 上下文可用性、窗口、缓存状态及 token 指标 |
| `NolaMac/Features/Inspector/MacInspectorWorkingContextView.swift` | 待发送/已发送附件、检索、跨会话记忆 |
| `NolaMac/Features/Inspector/MacInspectorDataSourcesView.swift` | 会话数据源 override 与恢复全局继承、全局策略边界 |
| `NolaMac/Features/Inspector/MacInspectorResponseView.swift` | 提示模板、回复风格、执行主机；风格不是 thinking effort |
| `web/app/templates/chat.html` | 旧 Web 的会话 CRUD、消息历史、流式投影、审批、附件、上下文、任务及设置面板 |
| `cli/src/web_parts/routes.rs` | 会话/消息/导出/分支/历史片段/上下文/goal/trace/session-memory/data-source 路由 |
| `cli/src/web_parts/native_bridge.rs` | v2 stream 请求、权限 profile 解析与执行桥接 |

## 3. 功能保留与修正矩阵

| 领域 | 已有事实 / 风险 | 完整替换必须达到 |
| --- | --- | --- |
| 会话与历史 | 旧 Web 有创建、搜索、重命名、删除、导出、分支、截断、消息编辑/重生成、分页、历史片段恢复 | 保留操作及明确确认；规范回执驱动结果；切会话取消/忽略过期读取，草稿按会话隔离；历史分页与实时尾部不重复、不丢消息 |
| 模型与执行单元 | 会话 PATCH 已支持模型选择、executionBackend 和各 backend 外部 session 映射；旧选择器会将不可用 backend 回落显示为 Nola | 明确显示保存值、有效值与不可用原因；不能静默切执行主机。会话选择不得写全局默认；外部执行单元不需要 Nola 模型时不强制模型校验 |
| Thinking effort / 参数 | Mac 从当前模型配置获取支持的 reasoning options；旧 Web 没有等价完整会话参数编辑合同 | Responses 只展示实际支持的 effort；其他协议的三档必须映射真实可用 effort/budget。继承、显式 override、恢复继承可区分；重开会话、切模型后有效值正确，不把无效旧参数发给新 provider |
| 回复风格 | 旧 Web professional/casual/playful 选择写全局 `/api/agent/settings`，不能冒充会话配置 | 回复风格与 effort 分开。跨端字段必须沿用 canonical 合同，不能把 Mac 展示枚举或浏览器私有值当统一会话字段；没有会话读写合同就诚实标注全局设置或暂不提供 |
| 权限模式 | Host stream 已接受严格的 `permission_profile`（safe/recommended/allow_all），缺省继承 agent settings；这是请求能力，不证明已有会话持久化 | 输入框下方独立小组件；展示继承及有效范围。会话模式先有 canonical 保存/恢复合同，再承诺跨重开有效；不能把全局目录/命令白名单写入伪装成会话授权 |
| 工具审批 | 旧 Web 有内联 pending 工具卡、一次/会话批准、拒绝和 guidance；审批依赖具体请求身份 | 按 Core 支持的 resolution 呈现，提交 exact nonce/request/host/client/session/tool 身份；回执后显示批准结果。参数变更/过期请求必须重审，不能批准最新同名工具替代旧请求；guidance 不改变调用参数且不等于批准 |
| 附件 | Mac 有 extracting/ready/failed、Quick Look；旧 Web 仅文本扩展名/文本 MIME、大小/数量限制、IndexedDB 与文本上下文封装 | 保留现有文本附件、移除/失败重试及会话所有权；处理中禁止误发；内容是不可信输入，不是权限指令。PDF/图片/Office 的上传、提取、受权预览必须接真实能力后才标可用；Finder/Quick Look 不复制成无效 Web 按钮 |
| 输入与发送 | 旧 Web 在持久化附件及提交完成前清空输入；发送结束总是滚底/聚焦 | 不透明输入框，光标/placeholder 对齐；中文 IME 不误发送，Enter/Shift+Enter 规则明确；提交前快照绑定会话及稳定 submission/message ID，失败保留或可靠恢复草稿和附件，不能覆盖用户随后输入 |
| 停止与恢复 | 本地 fetch 中断不证明 Host 停止；旧 Web 已有 cancel 与 trace 恢复入口 | 区分正在取消、已取消、连接断开、仍运行、完成/失败；取消调用真实 Host；重连恢复原执行，不重新提交同一用户消息，不把 EOF 当成功 |
| 消息主区 | 旧 Web 已消费 canonical 文本/reasoning/tool/trace/task 投影 | 文本、thinking、工具进度在稳定主时间线追加/更新，不在输入框上方先显示再搬运；稳定 identity + sequence 去重，处理 snapshot/delta/replay/terminal，不让历史读取覆盖更新投影 |
| Markdown / 资源 | 旧模板包含表格、代码、引用、数学/图示及资源处理入口；不能推定新 Markdown 组件已等价 | 表格、列表、中文强调、代码块、换行/段距、长路径可读；数学/图示有已验证支持或明确安全源码降级。复制与链接可用；禁止危险协议/HTML执行，不在 URL/DOM 泄露凭据；常用文件预览失败要解释而非空白成功 |
| 滚动 | 手动滚动、历史 prepend、stream 更新和 thinking 展开共同影响高度 | 只在用户跟随底部时自动跟随；离开底部后保留锚点并提供新内容提示；历史 prepend 保留可见位置；完成/折叠/resize/IME 不抢滚动和焦点 |
| 概览 | Mac 概览是展开的简短状态、上下文、工作上下文、数据源、回复配置 | 不折叠只有一行的信息；上下文放概览而非输入框常驻；unknown 不显示 0%；区分窗口/估算/实际 usage/缓存可用性。沙盒只显示名称及按需详情，不铺满绝对路径 |
| 工作上下文与知识 | Mac 有检索/跨会话记忆/数据源继承；旧 Web 有 session-memory、知识写入、全局记忆偏好设置 | 会话操作写会话、全局操作明确全局；取消/切会话后异步保存不得污染另一会话；来源读取错误独立可见。数据源 override 不绕过全局 blocked/Host 授权，恢复继承不同于关闭 |
| 活动/任务/目标 | 旧 Web 有 trace、任务状态、任务取消/指导、goal 控制及压缩来源恢复 | 保留可追溯真实执行记录、错误和终态；goal 操作遵循当前 Host 路由（当前为 GET/POST）；不凭计时器生成进度或成功；浏览器/外部执行状态缺能力时不显示假空闲/可控 |
| 临时会话 | 旧模板有 temporary 开关，但不创建列表会话不能证明 Host、trace、附件等不落盘 | 完成全链路存储/清理验收前不得承诺“无痕/不保存”；明确作用域，不能仅隐藏 sidebar 就宣称隐私语义已实现 |

## 4. 不能照搬的旧交互

1. 全局 `agent_mode_enabled`、`response_style`、开发工具目录/命令许可编辑器，不能标成“本会话”。模型连接密钥配置应指向统一模型/凭据设置，聊天页不再维护第二份密钥表单。
2. “普通聊天/Agent”按钮不能宣传未验证的独立运行通道；是否提供开关由实际执行合同决定，不能通过提示词或关键词假装切换架构。
3. 工具批准不能先显示“已允许，继续中”再等待服务器；确认中、回执成功、恢复失败是不同状态。
4. 文本附件读取不是通用文档支持；浏览器选到文件不是 Host 已获目录权限。应使用应用内授权确认，不能把系统文件选择动作作为 Core 许可替代品。
5. 上下文估算不能显示成 provider 精确用量，未知窗口不能硬编码占用率。前端动画只能表示正在工作，不能捏造执行阶段、百分比或剩余时间。
6. 安全错误提示不得直接渲染远端响应全文、认证 header、原始 credential 或可能含密钥的工具参数。必要诊断应经过脱敏并与用户可见本地化消息分离。

## 5. 需显式补齐的能力缺口

本次所读 `update_conversation_response` 只更新标题、模型选择、执行 backend 及外部 session 映射，不能据此宣称支持任意会话模型参数/effort/权限/回复风格持久化。新增能力必须同时核验请求、canonical 存储、重新读取和运行时应用；只增加 selector 或 stream 字段不算完成。

Mac 的本地工作区/Quick Look、平台数据源、浏览器执行能力和通用附件提取，不能因 Mac 已实现就推定 Web 已支持。Web 各能力的缺口需要逐项记录为“已实现”“明确不适用”“未完成”，包含实际 API/能力探测和回归证据。不存在的 API 不得用 localStorage 状态伪装后端结果。

2026-09-23 补充：Host 已有会话拥有的 PDF **预览资源**合同：`POST/GET/HEAD/DELETE /api/chat/conversations/{id}/resources[/resourceId]`，列表只给元数据；资源 ID 由 Host 生成，Store 外键绑定会话，8 MiB 单件、10 件/会话，校验 MIME、PDF 头尾与摘要，跨会话读取返回 404。`conversation_resource_e2e` 4 项 Store 测试及 `pdf_preview_resource_is_real_host_scoped_and_read_only` 真实 Host 测试通过。这不等于聊天提交已消费 PDF，也不等于 PDF 文本提取或图片/Office 预览；前端必须明确标为“仅预览”。会话删除由 canonical Store 拒绝活动 Run（409），成功后资源级联并清理进程 Host；原 `CloseSession` 的 SessionEnd 生命周期 hook 因 idle Core 不支持 Host effect 而未执行，后续需单独设计 hook 执行/回执，不能声称已完成。

## 6. Completion gates

每项验收记录至少包含版本、测试名/命令、结果及尚未覆盖的环境。下面均为待验收条件，不因本文存在而自动通过。

- [ ] **合同闭环**：请求/回执/投影的类型和身份一致；未知事件可安全保留/忽略并报告，关键错误不得吞掉；无前端第二套消息/权限事实源。
- [ ] **配置闭环**：会话显式设置优先于默认；缺省继承可见；切模型、切会话、刷新、另一客户端读取和下一次实际模型请求一致；不改全局默认，不泄露凭据。
- [ ] **权限闭环**：批准/拒绝/过期/重复点击/断线重放/变更参数覆盖；Core 最终裁决不被 UI 绕过；未支持的授权范围不展示。
- [ ] **生命周期闭环**：延迟读取、切会话、关闭组件、发送失败、附件失败、停止失败、重连全部不丢草稿、不串会话、不重复提交。
- [ ] **历史与渲染闭环**：上述旧 Web 操作可发现且有实际效果；历史/stream 顺序正确，Markdown 和资源安全降级可见，压缩内容可追溯。
- [ ] **真实交互闭环**：桌面与窄屏、深浅主题、七语言、键盘/IME、屏幕阅读标签、长消息与大表格、手动滚动/历史 prepend 经过浏览器验收。SSR 不算浏览器布局或 IME 验收。
- [ ] **测试分层**：纯 reducer/合同测试覆盖乱序、重复、断流、缺 snapshot 的合法 delta、stale response；实际 Svelte SSR 测试验证组件接线；浏览器验证真实交互；隔离 Host fixture 验证 mutation 回执。不得只测试复制的字符串或独立假实现。
- [ ] **真实模型验收**：在授权的隔离会话中记录模型/provider、会话标识、执行终态和恢复结果，覆盖工具审批及自然续问；多语言/歧义/未知工具恢复用于检验合同泛化，不能反向添加关键词路由。未运行就明确未验证。
- [ ] **切换记录**：逐项说明迁移能力、明确平台差异、剩余缺口与旧入口策略；禁止以“新页面已有输入框和消息列表”宣告完整替换。

## 7. 本次审计的限制

本次为源码只读审计并编写验收合同，未运行浏览器、真实模型、文件预览或权限交互。因此本文不声称现有或新 Svelte 页面已通过上述门槛；主集成验收应填入实际测试证据。
