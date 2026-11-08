> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# macOS 跨模块 UI 审计、设计与反思（2026-10-09）

状态：审计、设计反思、已批准模块实现及专项验收完成；最终两处范围/名称文案的定向验证也已通过，详见 §11 末尾。§1–10 保留各阶段原始证据，不代表当前仍未实现或待验证。整体聊天、模型与编译警告阶段分别以对应审计及总执行记录为准。

## 1. 范围与方法

读取根 AGENTS.md，并检查当前工作树的 Workspace、Sidebar、Composer、Inspector、会话参数面板、权限卡及 Settings；模型中心具体连接/模型域由独立审计负责，流式正文与滚动由另一审计负责。此次不修改源代码，不并行启动编译。

目标不是统一换皮，而是先明确“用户正在操作什么、状态属于谁、哪些事件可以改变界面”，再调整布局。沿用用户确定的约束：思考恒在正文上方，三档思考复杂度，权限属于执行操作而不是模型能力，概览不靠折叠藏内容，会话配置优先于模型/全局配置。

## 2. 模块清单与当前状态

| 模块 | 职责与现有优点 | 待整改边界 |
| --- | --- | --- |
| 工作区 Shell | 响应式三列/覆盖面板、列宽约束、减弱动态效果开关 | 根级动画事务可能传播至聊天；窗口装饰重复异步配置 |
| Sidebar | 导航与会话分组、搜索、重命名 | 导航切换会重建中央视图，瞬时状态应归属于会话/页面而非渲染实例 |
| Composer | 原生 TextKit、IME 防误发、不透明输入区、两项简洁快捷配置 | 附件额外成功行短时改变输入区高度；附件 VoiceOver 状态仍为原始英文 |
| Chat header | 主机、执行后端、当前会话模型选择 | 不说明全局默认与会话选择关系；会话参数入口缺失 |
| Inspector | 概览、知识、浏览器、工具，信息分区明确 | 后台浏览活动自动切 tab；概览混合运行事实与可编辑策略 |
| 会话参数 | 独立 draft、继承/覆盖、数值校验 | 面板不可达；协议归属和继承说明需要与模型中心统一 |
| 权限卡 | 明确一次/会话授权、拒绝、指导不等于授权 | 全请求内容被用作视图 identity，更新会重建本地交互状态 |
| Settings | 全局偏好、宿主连接、系统集成 | Provider 页重复模型摘要且不是观察式状态；历史草稿状态残留需核查删除 |

## 3. 有证据的优先问题

### P0：会话参数编辑入口断链

`rg isModelSettingsPresented NolaMac` 只有 `MacWorkspaceModel.swift` 的初值 `false` 和 `MacConversationCanvas.swift` 的 sheet binding。没有打开面板的 action。`MacChatWorkspaceView.modelSelector` 只有选择模型、刷新和管理模型，用户无法进入已实现的会话覆盖设置。

这是导航/所有权问题，不是参数组件美化问题。模型中心编辑的是可复用模型默认，聊天中需要明确入口编辑“仅当前会话”，不能把二者混为一个按钮。

### P1：后台事件抢 Inspector 当前页面

`MacChatInspectorView.swift` 的 `.macBrowserActivityDidChange` 收到后直接 `selectedTab = .browser`，且未区分主动导航和后台工具执行。用户阅读概览或配置知识时会被带走。应保留后台状态刷新但不改选择；只有显式浏览器操作/设置协助才能打开浏览器 tab。

### P1：权限卡身份与可变内容耦合

`MacConversationCanvas.swift` 对 `MacHostPermissionCard` 使用 `.id(request)`，而卡片拥有 `showsDetails`、`showsInstruction`、instructionComposer 等 `@State`。相同授权请求追加指导或内容更新会改变整个请求的值，从而销毁面板状态。旁边用户输入卡已正确使用 `.id(request.id)`。

应使用稳定请求 ID。请求实体可变内容只刷新显示；新请求 ID 才重置输入。保留现有 `expectedRequest` 授权竞争检查，不把 UI identity 的修改扩展成弱化权限验证。

### P1：输入区短暂成功提示引发布局升降

`MacComposerView.swift` 同时展示附件 chip 和额外 `attachmentConfirmation` 成功行；后者约 1.8 秒后动画删除，至少触发一次输入区增高、一次降低，连带 transcript 可视区域变化。成功状态已由附件 chip 中 checkmark 表示，额外行重复且导致跳变。

建议移除重复成功行，保留 chip 的 extracting/ready/failed 状态以及无障碍反馈。真实添加/删除附件带来的持久托盘高度变化是必要的，不应伪装成零布局变化。

### P2：窗口属性每次 SwiftUI 更新都排队写入

`MacWorkspaceShellViews.swift` 的 `MacChatWindowChromeConfigurator` 在 make/update 都 `DispatchQueue.main.async` 并重复写 title、styleMask、toolbar=nil 等属性，无附着/差异检查。这是潜在额外无效工作的证据，尚不能据此声称它就是用户闪烁的根因。

建议在 view 附着窗口时配置一次，后续仅更新真正变化的 title；不在正文 delta 驱动下反复重设窗口 chrome。需要窗口附着生命周期测试。

### P2：设置页模型摘要并非可靠默认入口

`NolaMacApp.swift` providers 设置以同步 singleton 查询 endpoints/configuredModels，然后 `selected ?? first` 显示“Default model”；这里既无 picker，也未建立针对模型默认变更的观察订阅。可能展示自动回退而非用户显式默认，且其他窗口更新后不保证即刻刷新。

统一由模型中心拥有全局默认写入；Settings 使用同一个观察投影展示真实来源并导航到同一中心，禁止建立第二套默认选择状态。具体默认域设计由模型中心审计提供。

### P2：屏幕阅读器状态未本地化

`MacComposerView.attachmentAccessibilityStatus` 返回 `extracting`、`ready`、`extraction failed` 原始文本，与 Inspector 已本地化的 Processing/Failed 状态不一致。应复用现有本地化键，不增加语言关键词判断。

## 4. 候选整体设计

### 配置归属

1. 连接拥有地址、认证、传输协议默认；模型拥有标识、可复用参数、能力声明；默认模型策略属于应用偏好而非列表顺序。
2. 聊天 header 表示当前会话实际使用的模型；模型菜单提供“当前会话参数”及“管理模型”两个明确入口。前者只编辑当前会话覆盖，后者进入模型中心。
3. Composer 只保留权限模式与三档复杂度；不新增 effort/budget 控件，也不再次放上下文计数。
4. Inspector 只展示当前会话相关事实与会话策略；后台事件刷新数据，不决定用户正在看的 tab。
5. 权限卡是一个稳定请求的交互实体：出现/结束可以改变布局；请求字段更新不能重建输入状态。

### 交互与动态规则

- 用户显式导航可以有短动画；流式数据、加载完成、终态合并不能造成整页过渡。
- 表单草稿与视图 identity 分离；跨对象导航才询问未保存变更，普通增量更新不重置草稿。
- 成功反馈优先在已有组件内改变状态，不插入几秒后消失的占位行。
- 全局默认变化不能覆盖现有会话显式选择；会话恢复时仍显示实际生效来源。
- 不用“全局禁用全部动画”替代定位布局/身份错误。

## 5. 设计反思（实现前）

### 风险一：增加另一个参数按钮是否又变复杂？

把高级入口放在现有模型菜单的次级分组，不增常驻控件；菜单文案明确当前会话范围。这个入口修复现有面板不可达，不新增配置体系。

### 风险二：取消自动切浏览器后是否看不到工具进展？

工具进展仍在 transcript 与当前工具 tab；浏览器 tab 需要时可显式打开。若后续增加未读点，它是提示而非抢焦点。不要为补偿该问题再新增通知弹窗。

### 风险三：稳定权限卡 ID 是否保留了过期批准？

UI 局部状态与授权命令不同。批准 action 仍携带最新 request 和 expectedRequest，Core 仍验证具体工具/资源；保留输入/展开状态不能复用旧授权结果。必须回归相同 ID 内容变化与不同 ID 切换。

### 风险四：去掉成功行影响可发现性？

保留附件 chip、加载与失败状态，成功 checkmark 足够。添加本地化 VoiceOver 状态。失败不能用瞬时提示消失。

### 风险五：大范围重构破坏成熟模块？

本轮将重构集中在上述有证据边界及模型中心域/页面；Data、Automations、Extensions 等独立内容不做无依据重写。统一导航/状态规则适用于它们，但不以“整体设计”为由替换所有实现。

## 6. 实现包与验收门

| 实现包 | 文件边界 | 验收 |
| --- | --- | --- |
| 会话配置入口 | Chat header + 现有 sheet | 菜单可打开；会话覆盖保存有效；不修改全局默认；运行中策略明确 |
| 稳定权限卡 | Canvas 单处 identity | 同 ID 更新保留草稿/展开；不同 ID 重置；授权竞争测试继续通过 |
| 非抢焦点 Inspector | Inspector 通知处理 | 后台浏览事件不切 tab；显式浏览协助仍可切 tab |
| 安静的附件反馈 | Composer | 添加/处理/失败/删除正常；无定时成功行高度跳变；IME、粘贴、拖入不回归 |
| Chrome 生命周期 | Shell representable | 初次附着生效；同值刷新不重配；新窗口重新配置；标题变化有效 |
| 默认模型联动 | 模型中心 agent 与 Settings | 全局默认、当前会话、连接/模型清晰；跨窗口更新；不通过 first 掩盖无默认 |

已有测试基础：`MacWorkspaceLayoutContractTests`、`MacWorkspacePresentationLifecycleTests`、`MacComposerRecoveryStateTests`、`MacPermissionCustomResponseTests`、`MacPermissionInstructionTests`、`MacChatWorkspaceBehaviorTests`。需要补充缺失的交互/生命周期断言，不能只检查纯布局常量。统一实现后再执行 Mac 定向测试与截图/运行检查；Rust/Swift 全面 warning 审计在前两项完成后进行。

## 7. 当前交付边界

这是当前源码审计与可执行设计，不是完成声明。未启动应用验证闪烁，未写实现，未跑编译。主任务需记录采纳/调整后的设计以及最终实现、验证证据。

## 8. 全工作区模块补充审计

本节补充实际读取 Data、Automations、Skills/MCP、System Access 的源码，不以聊天审计替代所有模块检查。

| 模块 | 源码证据与保留理由 | 具体整改与验证 |
| --- | --- | --- |
| Data Center | `MacDataCenterView` 已有 overview/sources/memory/usage 分区、数据服务单一拥有者、共享页面 header、刷新禁用与进度，内容不因刷新被整体清空；`refreshAll` 并行刷新平台与 Web 来源 | `navigationAction` 接收却未传给 `MacPageHeader`，窄窗口侧栏隐藏时缺少页面内恢复入口。补传动作；refreshButton 当前从文字标签换成仅 spinner 会缩短按钮，保留稳定标签/宽度。其他列表/数据域不重写 |
| Automations | `MacAutomationCenterView` 由 canonical store 拥有计划，编辑 draft 在 sheet、错误保留、删除明确说明历史保留；历史是有返回入口的单层子页面；搜索/状态筛选有空结果恢复 | 主页面同样漏传 navigationAction。修复共享 header 导航；不把编辑表单改成自动保存，不为一致外观破坏删除/运行边界 |
| Skills/MCP | `MacExtensionsCenterView` 采用来源筛选、搜索、gallery/detail、稳定 item ID，刷新时已有 items 保留；本机/连接 Host 数据在展示上可区分 | `refresh()` 只在 `.connectedHost` 拉远端，而 `.all` 首次切入会拉远端：在 All 点击刷新会把远端项目从列表删掉。统一查询意图为 `.all` 与 `.connectedHost` 都包含 Host；`load` 的 `guard !isLoading` 会吞首次本机加载期间的来源切换，需要合并最新请求/按查询 key 取消与代际隔离，而非随意并发覆盖。成功刷新应清理旧错误，失败刷新不能伪称空列表成功 |
| System Access | `MacCapabilityCenterView` 已有 runtime access 状态、按系统访问检查得出 ready/permissionRequired/optional；用户点击才 request；激活后 refreshAfterAppActivation 重检；使用共享 header 并正确传入导航；窄宽两种 row 布局 | 保留现有权限域和运行时校验，不把可访问状态等同于 Core 工具授权。Sidebar `MacWorkspaceSection.capabilities` 标题是 Capabilities，页面是 System Access，统一用户名称以免又被误认为模型能力。列表当前按状态排序，若权限变更引发主动行移位明显，可保留稳定类别顺序；此项需实测，不能预先归因为闪烁 |

### 8.1 导航与来源生命周期设计

- 一级页面统一通过共享 header 恢复侧栏；子页面返回父页面，不能把同一个按钮同时当返回与侧栏动作。
- Data/Automations 的导航遗漏是接线错误，可小范围修复；不新建路由系统。
- Skills/MCP 的筛选是查询意图，不只是最终数组过滤。UI 选择、加载请求与响应发布使用同一个来源集合契约；快速切换保持最新用户意图，旧请求不覆盖新结果。
- 已加载内容在刷新时保留，失败留错误与旧内容；真正成功返回空集合才显示空态。未配置远端 Host 有明确说明，不触发隐式配置修改。
- System Access 只管 macOS 系统访问，Capabilities/Profile 页只管模型协议能力；全工作区名称必须避免这两个领域混用。

### 8.2 补充反思

1. Data 的 Manage Sources 实际进入 Web Sources，而页面也展示平台数据来源。当前文案有范围歧义；先将该动作标明管理 Web 来源，平台权限继续进入 System Access，不新增重复来源设置。
2. Extensions 不能简单删除 isLoading guard 并开多个请求：那会使过期响应覆盖新的来源选择。采用一个最新意图驱动任务/代际模型；测试覆盖加载中切换和刷新失败。
3. 权限页面状态排序是有意把需处理项置顶，不能为了绝不跳动就无条件取消。先证明用户操作失焦/移位，必要时只在一次刷新批次后发布或使用稳定顺序。
4. 保留各内容域已经正确的保存、删除和权限语义；“整体一致”不是把所有页面变成同一种表单。

### 8.3 补充验收门

- 三列、双列、窄覆盖三种布局下，每个一级页面均可通过 header 恢复导航；子页面返回后搜索/筛选不被无关刷新重置。
- Data 刷新开始/结束不使 header 主要动作跳位置；平台/Web 状态刷新仍有效。
- Extensions All → Refresh 保留远端与本地；加载过程中 This Mac → Host/All 最终符合最后选择；Host 错误可重试，成功后旧错误清除；本机来源绝不被当成远端修改。
- System Access 从 macOS 设置回来重检仍生效；不引入自动授权或修改 Core 权限；侧栏与标题中英名称一致。
- Automation 新建/编辑失败保留草稿，删除仅影响计划且历史保留。已有 `MacCanonicalAutomationEndToEndTests`、`MacCanonicalAutomationRunEndToEndTests`、`MacCapabilityPresentationTests`、`MacExtensionContentTrustTests` 是回归基础，但导航与来源请求竞态需要新增测试。

## 9. 模型中心设计交叉评审

已完整阅读 `MAC_MODEL_CENTER_AUDIT_2026-10-09.md`，统一默认解析、profile identity、事务草稿、连接/模型/会话/权限分离方向一致。需合并以下细节，避免新入口反而制造歧义：

1. 文档顶部“新对话默认模型”与“现有继承会话随全局默认变化”不是同一语义。采用“默认对话模型”，补说明“用于未单独选择模型的会话”；不要告诉用户只影响新会话却改变现有继承会话。
2. 当前会话参数入口放在已有聊天模型菜单的次级分组，和“管理模型”分开；Composer 常驻只保留权限模式、三档复杂度。模型中心更改的是默认，聊天面板更改的是本会话。
3. “恢复模型默认参数”仅清空参数覆盖；“恢复默认模型”仅清空显式 profile 选择。两个动作不同，不能一键重置整个会话设置而不说明。
4. 全局默认不可用时，模型菜单/会话面板必须显示明确原因，不以默认 profile nil 重新落回第一个 API 模型；远端 Host 使用 Host 自身默认与来源信息。
5. 本次正在运行的模型配置冻结；全局默认更改后，界面若显示下次生效模型，需要区别正在运行的模型，不能把同一轮产出冒充新模型结果。
6. Settings 的只读摘要改为消费同一个观察投影并深链模型中心；删除旧的未使用 provider 草稿前先确认引用，不增加 UserDefaults 缓存制造第三个默认来源。

本补充仍为审计/设计，未进行实现或编译。

## 10. 第一阶段交互回归实现记录

主任务批准并修改了稳定权限 request ID、取消 Inspector 后台抢 tab、移除 Composer 临时成功行及附件状态本地化；这些生产文件由主任务维护。

本审计分工新增 `NolaMacTests/MacInspectorInteractionTests.swift`，实际承载 `MacChatInspector` 于 NSHostingView/NSWindow，通过无障碍树操作生产 tab 按钮并读取真实 Browser 内容，而不是仅测试另写的策略函数：

- 后台浏览活动不替换用户选中的概览。
- 显式浏览器设置协助能打开浏览器；用户再次点回概览后，后台活动不能夺回焦点。

模型使用隔离 canonical 数据库；Chrome 依赖注入无副作用快照，不触发系统扩展安装。本阶段未运行构建，等待统一测试批次。

权限卡 identity 的真正宿主测试仍需走生产 Canvas；不采用测试 wrapper 自行添加 `.id(request.id)`，否则无法捕获生产 `.id(request)` 回归。`pendingRemoteConversationID` 为 private，需由真实 remote permission event 注入，而不是放开生产字段可写性。

### 10.1 权限身份二次反思：ID 必须带作用域

后续审查不能证明 request.id 在不同 Host/客户端/会话之间全局唯一：`MacRemoteHostService.permissionRequest(from:)` 接收普通字符串 ID，没有 UUID 格式或跨 Host 唯一校验；Core `permission_requests.request_id PRIMARY KEY` 仅约束单个数据库。即使常见生成器使用 UUID，展示层也不能把概率惯例当成协议保证。

因此 `.id(request.id)` 比全值 request 稳定，但最终更严谨的设计应是类型化 `PresentationIdentity(hostID, clientID, sessionID, requestID, nonce, callID)`。指导记录、原因文案、消息和有效期不属于展示身份；相同挑战更新这些内容保留展开/输入。nonce 更新意味着新的挑战，或所有者/工具调用改变时，必须建立新的卡片局部状态。授权 action 继续携带最新完整 expectedRequest，身份调整不改变 Core 权限校验。

待统一测试退出后实施的回归：相同身份追加 guidance/更新 reason 身份不变；相同 requestID 分别改变 host/client/session/nonce/callID 身份不同；新 requestID 不同。配合真实 Canvas 宿主测试验证草稿保留/重置，不能只依赖 Hashable 相等测试证明 SwiftUI 状态行为。

### 10.2 第二次集中验证准备

`MacRemotePermissionRequest.PresentationIdentity` 与生产 Canvas 的 `.id(request.presentationIdentity)` 已落地，全部授权闭包继续传递最新完整 `expectedRequest`。独立身份回归覆盖 metadata/guidance 更新与 host/client/session/request/nonce/call 各维度切换。

`MacInspectorInteractionTests` 追加真实 Canvas 宿主回归：从生产 decoder 注入远端权限事件，再通过无障碍操作展开指导编辑器并输入草稿；同一挑战的指导文案更新必须保留展开和输入，新 nonce 必须重置。未放宽模型 private 会话归属字段，也未在测试 wrapper 另设权限卡 identity。

Inspector 宿主改为显式尺寸和 key window，无障碍遍历同时检查原始子元素及 unignored 子元素，并在失败时输出类型、标识、标签和值，供复核先前树为空的问题。以上宿主调整和新回归尚待第二轮集中执行，不能据此宣称交互验收通过。

第二轮证明仅调整窗口无效。独立 AppKit/SwiftUI 最小诊断进一步定位：必须在测试宿主启用公开 `accessibilityEnabled` 环境值；SwiftUI 的 AccessibilityNode 实现无障碍 selector 但不声明完整 NSAccessibilityProtocol，原协议强转会丢掉按钮。测试现使用动态 Objective-C selector 分发。多行 TextField 的原生 cell 未暴露 SwiftUI identifier，且直接写 AX value 不更新绑定，因此通过本地化 placeholder 定位实际 NSTextField，再用其 NSTextView editor 的 `insertText` 输入；最小诊断已验证原生内容与 SwiftUI binding 同时改变。未修改系统辅助功能设置或使用私有激活 API。完整产品回归仍待下一轮集中运行。

后续集中批次中 Canvas 草稿保持/新挑战重置与 Settings 两项回归通过；Inspector 显式切换仍因 LazyVStack AX 子树为空失败。独立只含 ScrollView/LazyVStack 的诊断复现同样行为，排除了产品通知与窗口遮罩干扰。现代 AXUIElement 自进程调用返回 apiDisabled，故测试专用 `MacHostedAccessibilitySession` 使用 AppKit 已公开的进程内 informal AX selector 设置 AXEnhancedUserInterface，并在每个测试作用域退出时恢复原始值；没有系统设置写入、TCC 授权或生产代码兼容层。Inspector 增加实际概览内容的正向断言，避免“浏览器内容不存在”单独造成假通过。该最后调整等待下一轮验证。

## 11. 模块完成复核与最终验收边界

本节以当前源码、`/tmp/nola-chat-sealed-reveal-finalbatch-20261009.log` 和 `/tmp/nola-chat-startup-unchanged-retest-20261009.log` 为准。前者执行 409 项、1 项明确 opt-in 系统选择器测试跳过、1 项 service 启动超时；后者在不改源码/超时门槛的条件下重跑对应 Journal 测试及相关项，3 项全部通过。不能把前者写成“409 项一次全绿”，也不因独立基础设施重试而忽略下表的实际通过证据。

| 已批准模块边界 | 当前实现与验证 | 结论 |
| --- | --- | --- |
| Canvas 权限卡 | `MacRemotePermissionRequest.PresentationIdentity` 包含 Host/client/session/request/nonce/call；Canvas `.id` 使用该身份，动作仍传完整 expectedRequest。身份 3 项、真实 Canvas 草稿保留/新 nonce 清空交互通过 | 已验收；未放宽授权 |
| Inspector | 后台浏览活动不更改 selectedTab；显式 Chrome 设置协助进入浏览器；真实宿主通过概览正向内容、浏览器内容及返回概览后后台事件断言 | 3 项宿主测试通过，其中一项为 Canvas；原 AX 空树问题已解决 |
| Composer | 临时成功行已删除；附件 processing/ready/failed 使用本地化；真实托盘添加后的高度不再定时变化 | `MacComposerRecoveryStateTests` 3 项通过；没有添加常驻高级参数 |
| Settings 默认摘要 | 复用模型中心 `MacDefaultChatModelPresentation`；订阅 canonical 默认和配置变化；精确导航 API profile，独立管理 API 入口明确选 API tab；旧 provider 草稿和 API 默认读取已删除 | Settings 2 项通过，覆盖跨窗口通知、不可用默认不改写、精确导航、本地默认不误报 API 错误 |
| Data / Automations header | 主页面补接 navigationAction；Data 刷新标签保留，18pt 图标槽位内切 spinner；子页返回动作保持独立 | 两种宽度真实 header 点击测试通过；既有刷新/编辑/删除域不重写 |
| Extensions 查询生命周期 | All 与 Host 均拉远端；generation 隔离旧结果，离开页面 invalidate；失败保留旧条目，成功清除错误 | loader 3 项通过，覆盖 All 刷新、快速来源切换和离开后的过期发布 |
| System Access | 页面继续在应用重新激活时 `refreshAfterAppActivation`；不自动申请权限，不把系统访问当 Core 工具授权 | 源码边界保留；名称最终补齐见下文 |
| 窗口 Chrome | 原生附着后配置一次；同窗口更新仅改 title，新窗口独立配置 | `MacWindowChromeLifecycleTests` 通过，包含后续 toolbar 不被重置 |

最后复核发现两项已批准设计遗漏，现已小范围补齐：

1. Data 的 `manage_sources` 仍显示“Manage Sources”，而 action 仅调用 `showWebSources`。现将按钮、帮助与无障碍文案改为“Manage Web Sources / 管理网页来源”，更新现有 key 的全部九种语言，不改变导航目标或平台权限入口。
2. Sidebar `MacWorkspaceSection.capabilities` 仍显示 Capabilities。现与页面、窗口、菜单统一使用既有 System Access 本地化键；快照窗口标题同步改名，不改 capability 类型、状态排序或授权逻辑。

对应源码为 `MacDataCenterView.swift`、`MacWorkspaceSelectionModels.swift`、`MacVisualSnapshotHarness.swift` 与 `Localizable.xcstrings`；既有本地化测试的中文期望及页面标题清单同步更新。`jq` 校验 catalog JSON/非空本地化通过，`git diff --check` 通过。最后两处文案后的编译和 `MacSecondaryPageCatalogCoverageTests`、`MacSecondaryPageLocalizationTests`、`MacSecondaryHeaderNavigationTests` 尚待主任务统一执行，不能以文案变更前的 409 项代替这一最终门。

聊天与模型的整体完成判定仍由各自审计负责，本模块复核不替代其流式绘制、性能、canonical 默认解析与事务验收。新增跟随底部问题已有真实 Journal append→seal、箭头操作、footer 重排和 640/1440 浅深色截图证据；详见聊天审计。自动化持久化/删除历史与系统权限回跳未作业务改动，保留既有契约；本次没有对用户真实系统权限做交互验证。

非阻塞后续项：Inspector 浏览器未读点、System Access 状态排序是否需稳定化，以及额外视觉润色。它们在初审中是条件性建议，尚无用户操作失焦证据，不作为此次必做范围，也不为“无跳动”取消必要的权限状态排序。除上述文案最终定向验证及主任务聊天/模型完成核验外，未发现本模块范围内仍缺失的已批准功能；本节不宣称 Rust/Swift 全仓警告阶段已完成。

最终文案验证已执行：`/tmp/nola-ui-completion-gates-en-20261009.log` 中 catalog 6 项、
localization 4 项、header navigation 1 项、secondary navigation 3 项全部通过。
该批另一个既有终态恢复测试因系统维护睡眠导致超时断言，未改源码/超时的中文补充
批次 `/tmp/nola-ui-completion-gates-zh-20261009.log` 3/3 通过，包括该恢复测试与两项
实际模型窗口渲染。上述最终文案门已收口，不将分批证据改写为首次整批全绿。
