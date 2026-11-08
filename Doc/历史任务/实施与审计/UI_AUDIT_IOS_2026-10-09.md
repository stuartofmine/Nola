> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola iOS 逐页面 UI 审查

日期：2026-10-09。对象：原生 SwiftUI/UIKit iPhone 与 iPad 客户端。目标是让聊天、授权和配置保持清晰、可撤销、可访问，而不是把桌面设置面板缩小后放到手机上。

结论：**Critical issues（代码层面存在可访问性缺口）**。主要问题集中在导航宿主复用、编辑事务语义、错误反馈位置和辅助功能，而非只缺少视觉装饰。本次不修改产品代码。

## 后续授权整改状态（2026-10-09，同日）

上面的结论和下方行号保留为整改前审计证据；用户随后授权实现，本节记录代码变化，不等同于设备验收通过。整组完成后由主代理统一解析、编译和测试，本分支未自行启动构建。

| 问题 | 本批实现 | 尚未闭环 |
| --- | --- | --- |
| I-01 | UIKit输入框启用Dynamic Type、每次更新同步trait字体、明确可访问名称 | 最大字号和中文组合输入设备验收 |
| I-02 | 模型页区分导航宿主；智能页恢复系统返回；智能/技能库/下载/通知sheet补Close | 全入口iPad/VoiceOver焦点实测 |
| I-03 | 两个日程详情改ScrollView、可选文字、Retry和重新加载状态清理 | 权限撤销→设置→返回流程 |
| I-04 | 新会话/行菜单具名、选中trait、来源Picker实际标签 | VoiceOver实际播报 |
| I-05 | 上下文停止700ms自动保存，Cancel重读持久数据，Save明确保存文本和知识，保存中禁编辑；更多设置删除误导性总Save并标明立即生效 | 现有记忆/上下文/知识三个存储API不是原子事务；部分成功后失败会保留sheet与错误、保留草稿和已更新的记忆基线，不宣称整体回滚；仍应统一存储事务 |
| I-06 | API保存改throw并在当前sheet显示错误；能力与Skill/MCP编辑新增丢弃确认；端点错误不再由空名称触发 | CAS/存储失败设备场景 |
| I-07 | 宿主连接与数据源验证保存Task并支持取消，await后检查取消再提交 | MLX测试UI stop未改，MLX底层不在范围 |
| I-08 | 健康大字单列、指标标签可换行、Chart字段本地化 | 图表完整AX描述与超窄屏验证 |
| I-09 | 聊天顶部从互相覆盖的ZStack改为单一HStack分配宽度 | 320pt/AX5/长模型名截图验收 |
| I-10 | 历史读取失败不伪装空态；智能读取失败保留旧结果并提示；清除失败提示；执行历史缺存储显示错误 | 服务重连/恢复交互 |
| I-11 | 测试参数、更多设置→智能改为导航push，数字键盘补Done；会话上限改显式草稿与State初始快照，继承默认值只修改草稿，脏状态返回提供保存/丢弃/取消；编辑页激活时移除父页Close，仅保留子页返回，外层sheet下拉同步保护；保存失败留在原页保留草稿 | 下拉在脏状态被阻止，确认通过返回按钮进入；原生返回、iPad与VoiceOver设备验收仍待完成 |
| I-12 | 学习反馈“用于未来”默认关闭；Core ingest必须显式返回ok:true才关闭，nil/错误/畸形响应均保留输入并内联提示重试；取消需确认丢弃，脏状态阻止下拉；明确本地记录/长期偏好/不重新生成回答边界；入口与选择按钮44pt和选中AX、输入框可访问标签与Reduce Motion；失败重试、响应契约和退出保护三项单测已在模拟器通过 | 实际sheet焦点、下拉和大字布局仍待设备验收。保存沿用同步本地Core接口，不宣称可中断磁盘写入 |
| I-13 | 思考完成保留当前展开状态，不自动移动阅读内容；历史消息仍初始折叠 | 完成时滚动锚点设备验证 |
| I-14 | 9pt通知badge改语义caption2 | 全色板明暗/高对比方案未改 |
| I-15 | Journal复用聊天Markdown；历史详情正文/思考复用Markdown，结构字段显示合法缩进JSON并保留64位整数，状态本地化、协议诊断下收；reload失败保留已有详情；新增公开read_conversation_export_range的iOS typed adapter，conversation/message/part/revision固定，原始content_json按8KB分页、重试、前后翻页、仅复制当前页说明；严格验证identity/revision/UTF-8 offsets/末页/大小并隔离取消及迟到响应；历史详情/分页10项模拟器测试通过 | 分页原始JSON只读，不自动解释为工具动作、授权或附件打开；常用文件的历史artifact预览尚未接入，I-15不算全闭环 |
| I-16 | API配置允许不填key，错误可见 | 连接发现/能力协商架构未在本UI批次重构 |

新增 `NolaTests/IOSUIInteractionPolicyTests.swift`：验证思考初始/完成展开策略、输入框动态字号设置与可访问名称、辅助功能字号trait同步；续补会话参数继承默认值仅改变草稿、无效输入仍属于未保存修改、范围校验三项策略测试。后续完整 build-for-testing 通过，iPhone 18 Pro Max 模拟器实际执行 7/7 通过；结果包 `Test-Nola-2026.10.09_9-14-19-+0800.xcresult`。这不是可访问性设备验收或完整导航交互测试。旧页面族未删除或接入。

同日 21:27 追加验收：完整 `xcodebuild test`（Nola scheme，现有 iOS 27 iPhone 18 Pro Max 模拟器，串行且仅 `IOSUIInteractionPolicyTests`）编译及 **10/10** 测试通过，0 failures；包含上述反馈恢复三项新测试。结果包 `Test-Nola-2026.10.09_9-26-37-+0800.xcresult`。测试没有验证实际反馈 sheet 的点击、VoiceOver 焦点或磁盘满场景。

同日 21:36 追加验收：完整 `xcodebuild test` 编译及 **20/20** 模拟器测试通过（`IOSUIInteractionPolicyTests` 10项 + `IOSHistoricalPartDetailTests` 10项）；xcresult记录0 errors、0 warnings、0 failed、0 skipped。结果包 `Test-Nola-2026.10.09_9-35-30-+0800.xcresult`。原文接口依据现有公开 `read_conversation_export_range`，不是新建Core接口；这一批验证typed query、回复校验及读状态模型，尚非原文阅读页的真实手势、文件预览或VoiceOver验收。

## 范围和证据等级

- 已索引 `Nola/UI/` 的 52 个源文件，检查 App/ContentView/MainTabView 根入口、SwiftUI 呈现声明与页面主体；以下登记 50 个页面、呈现变体或重要交互面。
- **C**：已读入口、主体和与结论有关的实现；不是全文件逐行证明，也不是运行通过。
- **L**：页面主体已检查，但在 `Nola/**/*.swift` 调用检索中未找到从当前产品根导航可达的入口，或只由另一未接入页面引用。不得当作线上可达窗口报错；先确认删除或接入。
- **P**：系统 UI 或内容桥接，只核查调用端契约。系统实际呈现、VoiceOver、键盘、截图和样式仍需运行验证。
- 本次没有启动模拟器、Xcode 构建或真实数据操作。所有屏幕的实际布局、动态焦点、黑暗模式和系统授权面板均未验收。没有把 JPEG 估计当作实际对比度数据。

## 逐页面和入口清单

路径均相对仓库根；行号指本次阅读时的位置。表中的“需验”并不表示已证明该页面失败。

| # | 页面或呈现面 | 实际入口与实现 | 状态及具体检查结果 |
| --- | --- | --- | --- |
| 01 | 根窗口、五个 Tab、iPad sidebar | `Nola/NolaApp.swift:251` → `Nola/UI/MainTabView.swift:23` | C；根 Tab 和自适应侧栏存在；通知深链另开 sheet，需验返回和恢复选中项 |
| 02 | 初次使用第 1 步，产品说明 | `OnboardingView.swift:129`，未完成 onboarding 的根分支 | C；滚动内容、进度、50pt 返回热区、Reduce Motion；无跳过整个引导入口 |
| 03 | 初次使用第 2 步，运行方式 | `OnboardingView.swift:161` | C；automatic/device/host 选择，不提前请求系统权限；需验 AX5 和说明多行 |
| 04 | 初次使用第 3 步，来源规划 | `OnboardingView.swift:199` | C；选来源不等于授权，默认 documents；需验选择状态的辅助功能播报 |
| 05 | 初次使用第 4 步，开始任务 | `OnboardingView.swift:241` | C；示例任务与权限说明；需验极短横屏底栏和正文关系 |
| 06 | 聊天主页面、窄屏历史抽屉、宽屏分栏 | `ChatView.swift:390` | C；主体、输入 dock、错误恢复、远端恢复和模型菜单；顶部 ZStack 重叠风险见 I-09 |
| 07 | 聊天空态、无模型、宿主离线 | `Chat/ChatEmptyStateCard.swift:31`，timeline 空态 | C；分离检查中/未配置/断线；建议卡行数固定，需验大字和长语言 |
| 08 | 对话历史列表及删除确认 | `ChatView.swift:920` → `ChatConversationsSheet.swift:129` | C；删除有确认；加载失败变空列表，图标无具体名称，见 I-04/I-10 |
| 09 | 消息正文、思考、工具、权限卡 | `Chat/ChatBubble.swift:50`、`:3321`、`:3739` | C；思考头44pt且有背景；单次授权优先；自动收起会覆盖阅读意图，见 I-13 |
| 10 | Markdown 表格、代码、数学及 Mermaid 子视图 | `Chat/ChatBubble.swift:1562`、`:1930`、`:2032`、`:2087`、`:2113` | C/P；表格代码独立横滚；表格无横滚提示且无列标题关联，数学桥字号固定17，需字体缩放和 VoiceOver 验证 |
| 11 | 历史原文详情 sheet | `ChatBubble.swift:179` → `Chat/IOSHistoricalPartDetail.swift:41` | C；加载/失败/重试/关闭齐全；展示字段名与状态来自协议，部分原始字段不能加载时只有说明，见 I-15 |
| 12 | 附件导入系统 sheet | `Chat/ChatComposer.swift:627` → `ChatDocumentPicker` | P；取消/成功回调、允许多选存在；需真机文档权限与键盘切换验收 |
| 13 | 附件 Quick Look sheet | `Chat/ChatBubble.swift:188` → `ChatAttachmentQuickLook` | P；调用端已检查，预览控制器内行为、不可预览格式和关闭需实测 |
| 14 | 文件作用域授权系统 importer | `ChatView.swift:794` | P；按请求身份回传选中或取消；iOS 必须保留系统授权机制，不能照搬 Mac 的纯内置授权 |
| 15 | 学习反馈 sheet | `ChatMessageTimeline.swift:288` → `ChatLearningFeedbackSheet:546` | C；medium/large + grabber；默认写入未来偏好、保存直接关闭，见 I-12 |
| 16 | 上下文 sheet：概览/知识/工具 | `ChatView.swift:587` → `Chat/ChatContextSheet.swift:57` | C；缓存、原文检索、知识与来源可见；取消/自动保存混用，见 I-05 |
| 17 | 会话设置 sheet | `ChatView.swift:672` → `Chat/ChatMoreSettingsSheet.swift:11` | C；会话/模型/全局作用域有标题，但保存只执行 memory 回调，见 I-05 |
| 18 | 会话模型上限子页 | `ChatMoreSettingsSheet.swift:261` → `IOSConversationGenerationSettingsView:684` | C；合法范围、继承和错误文案；numberPad 无键盘完成入口、返回无草稿保护，见 I-11 |
| 19 | 当前模型默认参数子页 | `ChatMoreSettingsSheet.swift:284` → `ModelParametersView.swift:24` | C；与模型中心复用，独立保存/重载冲突、草稿确认、键盘完成存在 |
| 20 | 技能库 sheet | `ChatView.swift:658` → `PersonalCapabilitiesLibraryView:655` | C；有搜索、错误、重试、导入；sheet 根缺少关闭，见 I-02 |
| 21 | 智能主 Tab | `MainTabView.swift:88` → `PersonalIntelligenceDashboardView.swift:5` | C；记忆、证据、能力入口；读取失败被清空和持续学习提示，见 I-10 |
| 22 | 智能面板 push/sheet 变体 | `SettingsView.swift:76`、`ChatBubble.swift:183`、`ChatMoreSettingsSheet.swift:124` | C；同组件固定隐藏导航栏，无返回/关闭，见 I-02 |
| 23 | Skills/MCP 能力库 push 变体 | `PersonalIntelligenceDashboardView.swift:227` | C；push 可使用父导航返回；删除确认、恢复旧结果后重试较好 |
| 24 | Skill 新建/编辑 sheet | `PersonalIntelligenceDashboardView.swift:790` → `PersonalCapabilityEditorView:1156` | C；大编辑区、字符数、保存校验；取消和手势可丢草稿，见 I-06 |
| 25 | MCP 新建/编辑 sheet | 同上，`.mcp` 分支 | C；URL 校验和 medium/large；无草稿保护；名称为空也显示端点无效，见 I-06 |
| 26 | 能力删除确认及导入错误 alert | `PersonalIntelligenceDashboardView.swift:795`、`:809` | C；破坏性删除标识存在；import cancel 进入 catch 的用户体验需验 |
| 27 | 数据首页 | `MainTabView.swift:78` → `HomeView.swift:253` | C；健康/日程/自动化卡和刷新；9pt通知徽标、重复 header/card 层级，见 I-14 |
| 28 | 数据来源中心 | `HomeView.swift:328`、`SettingsView.swift:61` → `DataSourcesView.swift:402` | C；四类系统来源、可读来源、最近使用、失败刷新 alert；需验父层 hidden toolbar 的继承表现 |
| 29 | 通讯录来源详情 | `DataSourcesView.swift:424` → `DataSourceDetailView(.contacts)` | C；读取状态、权限、策略自动保存、设置入口；空标签 Picker 见 I-04 |
| 30 | 日历来源详情 | `DataSourcesView.swift:443` → `DataSourceDetailView(.calendars)` | C；同组件独立分支，权限不足/读取错误；需验权限撤销后恢复 |
| 31 | 提醒事项来源详情 | `DataSourcesView.swift:462` → `DataSourceDetailView(.reminders)` | C；同上；当前操作与长期使用策略的作用域需截图核验 |
| 32 | 健康来源详情及 Dashboard | `DataSourcesView.swift:481`、`HomeView.swift:427` → `HealthDashboardView.swift:83` | C；无数据与错误独立；固定双列、单位截断及图表英文轴名，见 I-08 |
| 33 | 日程来源 hub | `HomeView.swift:431` → `DataSourcesScheduleHubView:772` | C；三入口清晰；联系人行嵌整体来源摘要，信息归属需重新核查 |
| 34 | 提醒事项详情 sheet | `HomeView.swift:443` → `ScheduleItemDetailSheets.swift:13` | C；关闭存在；不滚动且错误无恢复，见 I-03 |
| 35 | 日历事件详情 sheet | `HomeView.swift:451` → `ScheduleItemDetailSheets.swift:78` | C；关闭存在；长 notes/location 不滚动，见 I-03 |
| 36 | 新建/编辑数据源 sheet | `DataSourcesView.swift:1275/1284` → `URLDataSourcesView.swift:719` | C；来源类型、验证、策略、预览、草稿保护；验证时 Cancel 无响应，见 I-07 |
| 37 | 数据源文件/文件夹 importer | `URLDataSourcesView.swift:1040` 起 | P；区分 file/folder；系统取消、iCloud未下载文件、失效bookmark需实测 |
| 38 | 自动化列表、过滤和启停 | `HomeView.swift:435`、`SettingsView.swift:108` → `ScheduledActionsView.swift:16` | C；真实空态/过滤空态/加载失败分离；启停无需重新创建；长错误只显示两行 |
| 39 | 自动化执行历史 | `ScheduledActionsView.swift:692` → `ScheduledActionRunsView.swift:12` | C；搜索、状态、重试、加载分离；storage nil 被视作无历史，见 I-10 |
| 40 | 自动化执行详情 | `ScheduledActionRunsView.swift:102` → `ScheduledActionRunDetailScreen.swift:12` | C；概览、结果、时间、技术详情；ViewThatFits可保留；需完整内容可达性验收 |
| 41 | 执行结果 Journal sheet | `ScheduledActionRunDetailScreen.swift:40` → `JournalDetailSheet:557` | C；错误重试关闭齐全；正文 `Text(String)` 不渲染块级 Markdown，见 I-15 |
| 42 | 通知中心 push 与深链 sheet | `HomeView.swift:439`、`MainTabView.swift:59` → `NotificationCenterView.swift:40` | C；错误/重试/分页/标已读；sheet 无关闭，见 I-02 |
| 43 | 通知详情页 | `NotificationCenterView.swift:295/300/310` → `NotificationDetailView.swift:16` | C；已读撤销与来源信息；超长时间/来源行、异步错误、Undo可访问性需实测；旧SummaryDetail另列L-23 |
| 44 | 模型中心 | `MainTabView.swift:97`、`SettingsView.swift:90` → `ModelsView.swift:11` | C；无模型引导及local/cloud/host分组；被push仍另建NavigationStack并隐藏导航，见 I-02 |
| 45 | API配置与能力 sheet | `ModelsView.swift:91/109` → `IOSModelExecutionCapabilitiesView:653`、`APIModelConfigurationView:710` | C；API草稿保护存在，能力页没有；保存失败落父页、无key端点不能保存，见 I-06/I-16 |
| 46 | 宿主连接 sheet | `ModelsView.swift:155` → `WebHostConfigurationView:824` | C；验证身份后保存、inline错误、密码字段；连接中不能取消，见 I-07 |
| 47 | 模型参数、测试、测试内参数 sheet | `ModelsView.swift:86/103` → `ModelParametersView`、`IOSModelInferenceTestView:1039` | C；测试有stop和close，但再开参数sheet，改变后与正在运行测试关系不明，见 I-11 |
| 48 | 模型下载 sheet与本地测试push | `ModelsView.swift:149` → `ModelDownloadView.swift:20` → `MLXTestView.swift:18` | C；下载删除确认；下载根sheet无关闭；本地测试启动后无stop，见 I-02/I-07；未审MLX实现 |
| 49 | 设置、导出系统sheet、清空数据alert、隐私页 | `SettingsView.swift:14/248/254/203` → `PrivacyDisclosureView.swift:11` | C/P；清空有取消、导出禁重入；云端/宿主/学习的数据边界说明不够具体，见 I-12 |
| 50 | 非当前可达的旧页面族 | `ChatConversationsSheet:54`、`ModelPickerSheet:11`、`PermissionsSetupView:23`、`ScheduledActionBasicEditorView:11`、`ScheduledActionsOptionsView:13`、`MedicationSearchView:18`、`LogViewerView:17`、`BriefDisplayView:17`、`ModelTestView:13`、`ModelTestDetailViewNew:11`、`SettingsSheet:718`、`PromptTemplatesView:22`及新增/编辑/预览、`TaskConfirmationView:16`及Inline、`HomeSummaryDetailView:1250`、`CategorizedTodayView:808`及History/SummaryDetail | L；已读各页面主体/主要呈现部分但没有完整动态审查。旧模型选择行combine吞子按钮风险、任务金额静默解析失败、Medication无滚动、Brief固定浅背景、模板预览再叠编辑sheet均不应当作已上线可达bug；决定移除或重新接入后再验收 |

第50项明确是待清理页面族，不折算成“每个窗口已运行通过”。`DisplayFormatters`、`DataSourceUIExtensions`、`ScheduledActionRunText`、`ChatMessage`、`IOSConversationProjectionPresenter` 等不是独立窗口；只在本轮入口索引内，不声称完成数据层审计。

### 第50项展开：旧页面与子窗口的入口登记

以下登记项不加进49个当前入口组后宣称“所有窗口已实测”。L表示当前根入口未检出；子窗口的父入口存在于源码，但父页本身未接入。若后续保留这些实现，必须分别补运行验收；若删除，应一起删掉仅为它们服务的子页。

| ID | 页面/窗口 | 文件和呈现入口 | 已检查内容与待补项 |
| --- | --- | --- | --- |
| L-01 | 旧会话sheet | `ChatConversationsSheet.swift:54`，当前聊天实际使用`:129`的列表 | L；主体与删除路径；不要和当前历史列表混淆 |
| L-02 | 旧模型选择sheet | `Chat/ModelPickerSheet.swift:11` | L；选择、行内参数入口、关闭；行combine与嵌套按钮需VoiceOver核验 |
| L-03 | 旧模型选择内参数sheet | `Chat/ModelPickerSheet.swift:61` → `ModelParametersView` | L；读呈现契约；父sheet再开参数sheet，取消与参数提交需实测 |
| L-04 | 旧权限引导 | `PermissionsSetupView.swift:23/96`，仅Preview入口 | L；四类权限分页与完成页主体；并非当前Onboarding |
| L-05 | 旧权限引导失败alert | `PermissionsSetupView.swift:163` | L；确定/去系统设置；权限请求进行中仍可跳步的时序需验证 |
| L-06 | 自动化基础编辑 | `ScheduledActionBasicEditorView.swift:11` | L；编辑主体；取消、保存与持久化联动未运行验证 |
| L-07 | 自动化选项页 | `ScheduledActionsOptionsView.swift:13/60` | L；选项主体及子NavigationLink；父入口缺失 |
| L-08 | 药物搜索页 | `MedicationSearchView.swift:18`，仅Preview入口 | L；无滚动布局，大字和键盘遮挡风险；搜索错误与无结果需设备验收 |
| L-09 | 日志查看页 | `LogViewerView.swift:17`，仅Preview入口 | L；日志主体、筛选和导出；长行与敏感值需另做安全验收 |
| L-10 | 日志分享系统sheet | `LogViewerView.swift:118` | L/P；仅检查调用契约；系统分享取消和导出失败未实测 |
| L-11 | 简报显示页 | `BriefDisplayView.swift:17`，仅Preview入口 | L；固定浅背景，深色模式风险；不作为当前页面缺陷计入 |
| L-12 | 旧模型管理包装页 | `ModelManagementView.swift:11` | L；只包装当前ModelsView；不是另一套可达模型中心 |
| L-13 | 旧模型测试页 | `ModelTestView.swift:13` | L；测试主体、结果、错误；应和当前IOSModelInferenceTestView去重 |
| L-14 | 旧模型测试参数sheet | `ModelTestView.swift:358/718` → `SettingsSheet` | L；参数表单与退出；运行中修改参数的语义待验 |
| L-15 | 旧模型测试详情页 | `ModelTestDetailView.swift:11` → `ModelTestDetailViewNew` | L；测试主体及错误区域；不要将type名当文件名 |
| L-16 | 旧模型测试详情参数sheet | `ModelTestDetailView.swift:353` | L；内联表单与图标关闭；关闭可访问名称及热区需验 |
| L-17 | 提示模板列表及删除确认 | `PromptTemplatesView.swift:22/210` | L；列表、搜索、新建、删除确认；没有当前根入口 |
| L-18 | 新增模板sheet | `PromptTemplatesView.swift:197/342` → `AddPromptTemplateSheet` | L；表单和保存/取消；长模板、键盘、草稿丢失需验 |
| L-19 | 编辑模板sheet | `PromptTemplatesView.swift:201/479` → `EditPromptTemplateSheet` | L；表单主体；父列表与预览入口共享该页 |
| L-20 | 模板预览sheet及内嵌编辑入口 | `PromptTemplatesView.swift:205/621/661` | L；预览再开编辑sheet；应在单个导航宿主切换预览/编辑，避免层叠 |
| L-21 | 任务确认页 | `TaskConfirmationView.swift:16` | L；主体已检查；金额解析失败未给明确字段错误，重新接入前须修复 |
| L-21b | 任务确认Inline变体 | `TaskConfirmationView.swift:150` | L；内联主体已检查，不是独立窗口；取消及字段错误反馈需验 |
| L-22 | 今日分类页 | `HomeView.swift:808/888` → `CategorizedTodayView` | L；分类主体及历史详情入口；长条目需验 |
| L-22b | 历史条目详情 | `HomeView.swift:1196` → `HistoryDetailView` | L；详情主体与来源导航；长条目、返回路径需验 |
| L-23 | 旧摘要详情 | `HomeView.swift:1103` → `NotificationDetailView.swift:996`的`SummaryDetailView` | L；该摘要分支不是当前通知中心路由；来源详情子NavigationLink在`:1173` |
| L-24 | 首页旧摘要详情 | `HomeComponents.swift:1250` → `HomeSummaryDetailView` | L；详情主体；未找到当前调用，避免误算成已可达窗口 |

L-04中的通讯录、日历、提醒、健康四个授权步骤共用`StepView`，完成页为`CompletionView:326`；已读共用主体与各步骤配置，系统授权弹窗仍为P，未做真机授权操作。附表26行登记，不等于26个独立且可达的窗口。

## Critical

### I-01 输入框 UIKit 字体不会随运行中的 Dynamic Type 变化更新

- 证据：`Nola/UI/Chat/ChatComposer.swift:110` 仅在 `makeUIView` 设置 `.preferredFont(forTextStyle: .body)`；`:119` 的 `updateUIView` 不更新字体，也未设置 `adjustsFontForContentSizeCategory`。外层 hint 是 SwiftUI `Text`，两条字号路径不同。
- 影响：用户改变系统字号后，主输入内容和 hint/周围正文可能失配；这是实现缺口，实际错位幅度需模拟器验证。
- 修法：启用自动字号响应，在字体变化后失效布局并重新计算六行高度；placeholder 与 UITextView 用同一 text container 或同一字体/Insets 数据源。给编辑器明确的可访问名称，而不只加 identifier。
- 依据：`typography.md › Supporting Dynamic Type`：“Make sure your app’s layout adapts to all font sizes.”

### I-02 多入口页面隐藏返回或没有显式关闭

- 证据：`PersonalIntelligenceDashboardView.swift:81` 固定 `.toolbar(.hidden, for: .navigationBar)`，却既是 Tab、又由 `SettingsView.swift:76` push、由 `ChatBubble.swift:183` 和 `ChatMoreSettingsSheet.swift:124` sheet。该组件未提供 dismiss。`ModelsView.swift:47/78` 内建 NavigationStack并隐藏栏，却被设置页push。`ModelDownloadView.swift:70`只有title，无关闭；其sheet入口在 `ModelsView.swift:149`。`NotificationCenterView.swift:189`工具栏只有筛选/已读，深链sheet在 `MainTabView.swift:59`。技能库sheet在 `ChatView.swift:658`，库view同样无dismiss。
- 影响：没有可见的返回/退出动作，只依赖系统手势或系统可能提供的外部手段；无障碍、iPad和深层导航尤其脆弱。不是断言系统绝对无法退出。
- 修法：把页面内容与导航宿主拆开；由Tab根、push、sheet分别决定标题和Close。push不要再包新NavigationStack；所有sheet根都有明确关闭按钮。
- 依据：`accessibility.md › Mobility`：“Offer alternatives to gestures.”；`sheets.md › Anatomy / Best practices`。

### I-03 日历和提醒详情不滚动，长内容和大字无法保证可达

- 证据：`ScheduleItemDetailSheets.swift:20`、`:85`在NavigationStack内放Group/VStack/Spacer，title、notes、location任意长度，没有ScrollView。
- 修法：内容区ScrollView，正文支持选择；保持Close可见；添加不存在、权限撤销状态下的重试/去设置入口。用超长notes、AX5和横屏做必测。
- 依据：`layout.md › Adaptability`：“Be prepared for text-size changes.”

### I-04 高频独立操作的无障碍名称与选中状态不明确

- 证据：`ChatConversationsSheet.swift:155`新建按钮只有plus图标；`:292`行菜单只有ellipsis且没有对话名称；`:264`选中状态只体现在色条/背景，未声明 `.isSelected`。`DataSourceDetailView.swift:129`使用 `Picker("",...)`并`.labelsHidden()`，旁边独立Label不是Picker的可访问名称。
- 修法：新建按钮明确名称；行菜单“更多操作：对话标题”；选中对话加selected trait；Picker使用真实title再隐藏视觉标签。不要依靠SF Symbol的系统通用读法表达操作意图。
- 依据：`accessibility.md › Vision`：“Describe your app’s interface and content for VoiceOver.”

## High

### I-05 配置和上下文的编辑事务模型与 UI 承诺不一致

- 证据：`ChatMoreSettingsSheet.swift:88/96`同时展示Close/Save，`:654`保存只调 `onSaveMemoryProfile()`。`ChatView.swift:702` permissionMode绑定setter立即持久化，`:728` responseStyle同样立即变更；关闭并非放弃修改。`ChatContextSheet.swift:692` Cancel仅dismiss；`:707`对知识/百科变化安排自动保存，`:763`等待700ms，`:716`离开取消任务。
- 影响：同一个“保存”不控制全部配置；“取消”可能已经写入部分内容，也可能丢掉最后700ms；重开时绑定中的未保存草稿仍可能留在父页。
- 修法：选一种明确模式：设置页即改即存则去掉统一Save并逐项显示错误/范围；文档式编辑用独立draft+revision，Save一次提交，Cancel完全回滚。自动保存需明确状态且离开前flush，不由UI销毁悄悄取消最后一笔。
- 依据：`sheets.md › Anatomy` 对Cancel/Done语义；`generative-ai.md › Best practices`：“Keep people in control.”

### I-06 编辑器保存失败不可见和草稿丢失

- 证据：`ModelsView.swift:130` API保存异常写入父页面`statusMessage`并返回false；API sheet没有对应错误区，因此留在sheet的用户看不到原因。`PersonalCapabilityEditorView:1233` Cancel直接dismiss且无dirty保护；能力配置页 `ModelsView.swift:653`同样；MCP `:1223`依据同时校验名称和URL的`isValid`展示URL错误，名称为空也会误报地址。
- 修法：保存回调改throw或返回typed result，由当前editor显示错误；各draft统一未保存确认；校验拆成按字段结果，Save禁用时说明哪个字段无效。
- 依据：`entering-data.md › Best practices`：“Dynamically validate field values.”；`sheets.md › Mobile (iOS, iPadOS)` 未保存修改确认。

### I-07 网络/验证长任务把用户困在当前面板

- 证据：`ModelsView.swift:951`连接期间 `.interactiveDismissDisabled(isConnecting)`且Cancel disabled，`:990`启动未保存句柄的Task。`URLDataSourcesView.swift:1017`验证时禁止手势，`:1073` Cancel走`guard !isValidating else { return }`但按钮仍像可用。`MLXTestView.swift:125`本地测试仅start，运行时disabled，没有停止。
- 修法：保留Task句柄和明确Cancel，取消推进UI/请求token而不写入半成品配置；验证/测试不能退出时至少明确状态和可终止策略，但不要把无法取消当作理想方案。
- 依据：`generative-ai.md › Best practices / Outputs` 的控制权和延迟反馈；具体取消实现为设计判断。

### I-08 健康面板在可访问字号下仍固定双列并截断单位

- 证据：`HealthDashboardView.swift:87`两个固定flexible列；`:133`所有尺寸使用这些列；`:226`数值下的标题/单位`.lineLimit(1)`。`:147`图表语义轴使用英文`Day`/`Steps`。
- 修法：AX字号切单列、允许名称/单位多行；为每个数值提供合并的名称/值/单位辅助信息；本地化图表语义轴、补图表可访问数据描述。需实测AX3/AX5和iPad窄分屏。
- 依据：`typography.md › Supporting Dynamic Type`：“Keep text truncation to a minimum as font size increases.”

### I-09 聊天顶部菜单与左右动作没有共同宽度约束

- 证据：`ChatTopBar.swift:53` ZStack中心模型菜单；`:105`中心最大宽度196/AX240；`:120`另叠一层HStack侧栏、执行单元、设置。`:181`执行菜单至少38宽，页面横向padding24。
- 风险：320pt宽且左右动作齐全时，中心菜单不会为侧边热区自动让位；AX字号240中心宽加两边44pt按钮和padding已超过320，另有执行单元时更差。
- 修法：单一布局分配leading/trailing列和中心剩余宽度；AX或紧凑宽采用标题第二行/执行单元归入菜单。验收不得只看常见宽屏截图。
- 依据：`layout.md › Adaptability`。这里是几何约束风险，未声称已经点击实测重叠。

### I-10 失败被展示为“没有数据”或“持续学习”

- 证据：`ChatConversationsSheet.swift:317`catch后把conversations清空，随后出现“还没有对话”；`PersonalIntelligenceDashboardView.swift:606` snapshot nil回退空字典，但`:145`仍显示Learning continuously；`:97`清除返回值被忽略。`ScheduledActionRunsView.swift:160`storage缺失也清空并置loadError=nil。
- 修法：显式loading/loaded-empty/failed/stale状态，失败保留上次成功结果；显示重试。清除失败必须可见，学习状态来自运行状态而不是固定文案。
- 依据：`generative-ai.md › Outputs`：“If something goes wrong, describe what happened in plain language and offer a clear next step.”

### I-11 模态层级和数字编辑退出路径不统一

- 证据：`ModelsView.swift:103`模型测试作为sheet；测试`:1116`再开参数sheet。`ChatMoreSettingsSheet.swift:124`设置sheet再开智能sheet。`IOSConversationGenerationSettingsView:721/727`数字键盘无FocusState/keyboard toolbar，页面无dirty保护。相比之下`ModelParametersView.swift:118`已提供键盘Done。
- 修法：sheet内详情用同一NavigationStack push；会话上限编辑复用已成熟的数字draft和键盘Done；离开未保存数据需一致策略。正在测试时是否允许编辑后续测试参数应明确标注。
- 依据：`sheets.md › Best practices`：“Display only one sheet at a time from the main interface.”

### I-12 长期学习和数据外发边界说明不足

- 证据：`ChatLearningFeedbackSheet:553` `appliesToFuture = true`，说明却强调only when you allow it；保存`:640`直接提交并dismiss，回调无结果反馈。`PrivacyDisclosureView.swift:27`三条通用说明未具体列出宿主工具结果、MCP与附件上下文可进入选定模型。
- 修法：默认纠正当前回答，长期偏好明确opt-in；保存回调返回成功/失败，失败留在编辑页。隐私页按On Device/Host/Remote API分别说明会发送的类别、目的、可关闭位置，不编造provider训练政策。
- 依据：`privacy.md › Best practices` 的精确控制；`generative-ai.md › Privacy`：“Clearly disclose how your app and its model use and store personal information.” 默认选项是本审查判断，不能推出当前存在未授权传输。

## Medium

### I-13 完成事件强制收起用户正在看的思考

- 证据：`ChatBubble.swift:3825` isFinished变true且已展开就自动收起，没有区分默认展开与用户手动展开。
- 修法：显式“用户已操作展开”状态，生成完成只改变状态图标；不覆盖用户展开决定。避免收起高度突变干扰滚动跟随。
- 依据：`generative-ai.md › Best practices` 的控制权；具体展开策略为设计判断。

### I-14 颜色和徽标仍没有统一可访问性契约

- 证据：`DesignSystem.swift:24`起品牌与signal均固定RGB，没有light/dark/high-contrast variants。按sRGB公式，primary与纯白对比为3.63:1，success为3.18:1，warning为2.15:1，danger为3.91:1；这些是色板与纯白的计算，不是所有实际按钮的最终对比度。`HomeView.swift:309`通知徽标文字固定9pt，低于参考的11pt移动端最小字号，虽然按钮另有可访问数量。
- 修法：为文本用途与装饰用途拆token，以实际surface逐项测量；浅色小号warning文字不要直接用当前色；数字badge用可缩放caption2和自适应边界，不把固定9pt当品牌风格。
- 依据：`color.md › Best practices`：“Make sure all your app’s colors work well in light, dark, and increased contrast contexts.”；`accessibility.md › Vision`。

### I-15 同一内容在不同详情页呈现能力不一致

- 证据：聊天有独立Markdown解析，`ScheduledActionRunDetailScreen.swift:591`却用`Text(journal.content)`；SwiftUI动态String不会进行同等块级Markdown解析。历史详情 `IOSHistoricalPartDetail.swift:64`直接显示协议fields keys，deferred仅显示说明，没有明确原文获取动作。
- 修法：可复用只读Markdown文档渲染器，仍保留原文复制；技术字段名折到详细信息，普通用户先看本地化内容与状态。deferred可提供明确来源/重试/原文读取路径，但不可自行扩大读取权限。
- 依据：`layout.md › Visual hierarchy`；具体内容组件共享为设计判断。

### I-16 iOS 模型配置还没有跟上连接级交互

- 证据：`ModelsView.swift:810`空apiKey直接禁止Save，对合法无key局域网端点没有表达方式；API配置只有手输模型名，无发现模型/连接测试；能力页`:669`仍提示Unknown abilities remain off并逐项开工具/采样。
- 修法：连接编辑器建模为地址/协议/可选认证/发现与手动模型；认证要求由端点策略表达，不能用空字符串验证代替。能力属于模型协商或override，不与用户工具授权混为同一组开关；未知状态与不支持分开呈现。
- 依据：`entering-data.md › Best practices` 提供选择并及时验证；连接级结构为产品架构判断。

## Craft 和应保留的实现

Nola的合理特色应是“当前工作状态清楚、授权可理解、原始证据可追溯”，而非每个设置都做一张带图标的大卡。现有多个NolaPageHeader+Hero+SectionTitle堆叠应按页面目的删减，但具体密度要拿截图判断；本轮不把个人视觉偏好伪装成Apple硬要求。

应保留：平台Tab/NavigationStack、正文语义字体、输入附件独立44pt删除、思考44pt标题与独立背景、权限单次优先、系统文件选择器、对敏感参数的redaction、模型参数快照冲突与重载、数据源真实验证预览、运行历史错误与过滤空态分离、受Reduce Motion控制的大部分过渡。

## 必须补的运行验收矩阵

1. 最窄支持iPhone、横屏短高度、iPad窄分屏和宽屏：每个C页面走一次进入/返回/关闭；每个P系统sheet走成功/取消/失败。
2. 默认字号/AX3/AX5、VoiceOver、Switch Control：所有独立按钮名称、选中值、展开值、焦点返回、表格列和代码复制。
3. 简中/英文/至少一长文本语言：错误文案、picker、授权范围、日历标题、数字单位、图表语义。未做全localization key完整性证明。
4. light/dark、Increase Contrast、Reduce Motion、Reduce Transparency：使用实际渲染色取样，不把上述纯白色板计算当成验收。
5. 中文组合输入、numberPad、硬件键盘：hint对齐、6行输入上限、键盘遮挡、Done/Cancel、附件sheet和焦点恢复。
6. 慢网/断网/失效凭据/权限撤销/源文件不可达/重放大历史：错误不得替换成空态，取消不丢状态、保存只提交当前draft，长期记忆写入可追踪。

未完成的工作：动态交互与视觉验收全部待做；第50项旧页面族需要先完成入口去留决策；native preview/系统授权/文件浏览器只审查了调用契约。不能将本报告称为“所有窗口已经通过审查”。
