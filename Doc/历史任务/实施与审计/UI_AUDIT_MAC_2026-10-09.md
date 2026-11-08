> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola Mac 全窗口 UI 审查 2026-10-09

## 结论与范围

### 2026-10-09 整改实施状态

本节记录审查后的产品修改；下文保留原始发现与证据，行号以审查时为准。实施不等于窗口实测通过。

- M00：已提高报告列出的业务微型字号，公共指标、数据契约、健康单位、记忆元信息、代码文件引用等保持至少10pt，常用辅助文字11pt；未机械放大装饰图标。
- M01/M02/M03/M14：提示词、活动、紧凑通知选择改原生Button；工具详情使用独立原生展开按钮，不吞掉文件链接；数据源开关和连接字段有标签；健康图表保留逐日读屏数据；追问选项增加勾选符号与selected语义。完整VoiceOver及纯键盘操作待实测。
- M04/M05：记忆保存中禁重复提交与编辑，编辑器呈现保存失败；非空列表也显示操作错误；删除防重复；搜索零结果不再冒充从未有记忆。
- M06/M07：上下文限制明确回车提交及合法范围；记忆入口直接打开memory-center。
- M08：Web源编辑持有取消Task，测试期间为“停止测试”；取消保留草稿，store在网络返回与持久化之间检查取消，不记录假连接失败。测试中锁定表单，避免提交旧草稿后关闭并丢掉新输入。
- M09：Quick Look翻页同步原生窗口标题，上一/下一文件按钮有可读名称与Command-方向键。
- M10：自动化、新模型、会话参数编辑加入dirty退出确认和交互关闭保护；独立模型参数窗口增加原生close delegate代理，保护红色关闭与Command-W并转发原窗口delegate。后续补齐扩展详情：工作台常驻/紧凑侧栏换页、自定义返回、原生关窗共用同一个退出决策，导航仅转发意图，不复制草稿或dirty状态；取消保留草稿，明确放弃后才执行原目标，保存期间不退出、不延后偷偷导航。删除与恢复内置内容也经同一草稿保护。2026-10-09 21:26 本组真实 Mac 测试 10/10 通过（新增扩展退出5项、原窗口关闭2项、删除/保留文件/删除中发送3项），xcresult：`Test-NolaMac-2026.10.09_9-25-47-+0800.xcresult`。真实窗口键盘/鼠标验收仍未完成；这不是全局编辑会话框架已完成的声明。
- M11：新增模型sheet改为可伸缩区间，浏览器预览最小尺寸降低，权限动作采用横/纵自适应布局；各尺寸截图验收未执行。
- M13/M18（本组部分）：健康反馈保留到用户关闭，遵循减少动态效果；参数保存动画遵循减少动态效果。
- M16：非法数字/种子提供文本错误和合法范围；会话参数Cancel支持Escape和未保存确认。
- M17/M19（协作部分）：扩展/自动化文件夹选择适配异步系统sheet；Workspace菜单补模型测试、数据管理、健康和药物查询入口。其他根代理整改见综合报告。
- M12：会话删除先确认，持久化失败保留列表项并显示错误，防止重复删除；删除记录不再顺带清理生成文件。重命名持久化成功后才更新展示，失败保留草稿。
- M12 续补：删除等待后端回执时显示进度，composer 与 send 准入同时阻止提交，失败恢复发送且不消费草稿；新增删除/发送竞态回归。
- M13/M18（补充）：数据管理的重要操作反馈不再定时消失；侧栏搜索悬停动画遵循减少动态效果。
- M15：工具页不再由执行位置推断已启用/已授权；显示真实浏览器连接状态或“使用时检查”，明确连接不等于授权。完整 Core 工具可用性投影仍未实现。
- M17（补充）：工作区文件选择和诊断导出使用异步原生面板，不再阻塞 `runModal`；选择返回时检查会话归属。

后续验证：完整 Mac 编译、链接通过；修复 helper 安装造成的签名加载失败后，普通 Xcode 构建和 9 项定向回归通过，不再依赖手工重签。结果包 `Test-NolaMac-2026.10.09_9-16-30-+0800.xcresult`。8 张模型中心及思考配置明暗色/尺寸截图已查看，夹具中未见文字或控件重叠；这不是所有窗口及 VoiceOver 验收。详细边界见综合报告。

新增回归：`MacEditorWindowCloseGuardTests`覆盖未保存拒绝关闭、允许关闭及原delegate/edited状态恢复；`MacWebSourceE2ETests.testCancelledValidationDoesNotCreateSourceOrFailureContract`覆盖取消不写源、不记失败、不发网络。实际通过范围见上方验证记录，不由代码存在推断。

评级：Critical issues。Mac 端已经形成聊天工作台、模型管理、数据与自动化的基本结构，但可访问性、编辑事务、取消语义和破坏性操作尚未形成统一契约。最紧迫的不是增加装饰，而是让每个窗口中的选择、保存、失败、取消都可靠且可发现。

本报告是 SwiftUI/AppKit 代码审查，不是运行截图或 VoiceOver 实测。检查了 `NolaMacApp` 的 **19 个 Window/WindowGroup 声明、1 个 Settings 声明**，以及 `MacQuickLookController` 的独立原生预览窗口，共 **21 个顶层窗口入口**；下文另列子页、sheet、popover、对话框和系统面板。相同组件在独立窗口和工作台内重复呈现，不重复算作实现。没有运行 Xcode 构建，没有启动会写入用户数据的测试。

设计主张：Nola 应是一个可随时观察、干预、恢复的本地工作台；内容优先，执行状态与授权明确，系统控件负责基础交互。当前最鲜明的是对话与执行证据并置，但零散的卡片、微型文字和重复管理入口削弱了这一点。

参考依据为本地 apple-design 中已读取的 `accessibility.md`、`layout.md`、`typography.md`、`color.md`、`designing-for-macos.md`、`windows.md`、`keyboards.md`、`sheets.md`、`alerts.md`、`privacy.md`、`settings.md`。其中 macOS 不支持 iOS Dynamic Type，不能直接用移动端字号机制代替 Mac 字体缩放设计。下文未给未经测量的对比度数值，也不将静态代码不能证明的运行时故障写成已复现。

严重度采用 apple-design 的口径：代码可确定的无障碍契约缺失列 Critical；这不等于已经用设备证实全部辅助技术行为。尤其 M01/M02 的系统隐式语义、焦点与 VoiceOver 播报仍待实测，报告所确认的是原生选择/动作与显式标签契约缺失，不将可能存在的系统推断算作已验证。

## 顶层窗口逐项清单

状态“代码已查”表示已读入口、主体视图、关键交互及相应风险分支，不代表每条状态路径已实测。所有行的实际尺寸、键盘循环和明暗模式仍须运行验收。

| 编号 | 窗口与入口 | 实现与检查结论 | 后续验收重点 |
| --- | --- | --- | --- |
| W01 | 主工作台 `NolaMacApp.swift:383` | `MacChatWorkspaceView`、Sidebar、Canvas、Composer、Transcript；代码已查 | 最小 920×640、三栏转覆盖层、流式结束位置、焦点回到输入框 |
| W02 | 独立会话 `NolaMacApp.swift:405` | 同一工作台实现；按会话 ID 创建窗口；代码已查 | 两个窗口同时打开同一会话，通知路由是否重复开窗、草稿焦点归属 |
| W03 | 数据中心 `:419` | `MacDataCenterView.swift:80`；概览、数据源、记忆、使用记录及健康/Web 下级路由；代码已查 | 数据契约字号、长路径、源失败/未测试与就绪区分 |
| W04 | 长期记忆 `:428` | `MacMemoryCenterView.swift:50`；搜索、新建、修订、删除；代码已查，见 M04/M05 | 已有记录时操作失败、慢保存连点、零搜索结果 |
| W05 | 系统访问 `:446` | `MacCapabilityCenterView.swift:22`；自适应行、权限说明、回前台刷新；代码已查 | OS 拒绝/限制/授权中；不可只因进程位置声称已授权 |
| W06 | 自动化 `:455` | `MacAutomationCenterView.swift:44`；搜索/过滤、运行历史、编辑和删除；代码已查 | 编辑后取消、长任务名、运行中删除说明 |
| W07 | 活动中心 `:464` | `MacActivityCenterView.swift:60`；列表、详情、清空；代码已查，见 M01 | 纯键盘选择、紧凑布局、刷新保留当前选择 |
| W08 | 模型中心 `:473` | `MacModelCenterView.swift:158`，远程/本地管理；代码已查 | 各端点独立草稿、窗口关闭与导航离开保护、加载失败恢复 |
| W09 | 工作区变更 `:484` | `MacWorkspaceReviewView.swift:51`；文件列表、diff、提交、push；代码已查 | 底部提交操作可达、外部 Git 错误、超长 diff、窗口关闭时草稿 |
| W10 | 扩展中心 `:492` | `MacExtensionsCenterView.swift:106`；目录、详情、Skill 编辑、Chrome 管理；代码已查 | 编辑草稿离开、远端不可用、本地/远端错误归属 |
| W11 | 诊断 `:501` | `MacDiagnosticsCenterView.swift:296`；持续刷新/暂停、过滤、导出/清除；代码已查 | 保留选择与滚动、长日志、导出面板的跨窗口行为 |
| W12 | 数据管理 `:510` | `MacDataManagementView.swift:36`；备份、恢复预览、清理；代码已查 | 操作中关闭、失败保留记录、4 秒消失反馈，见 M13 |
| W13 | 提示词库 `:519` | `MacPromptLibraryView.swift:33`；列表、详情、导入导出、编辑；代码已查，见 M01 | 纯键盘、空过滤结果、保存失败与多窗口编辑 |
| W14 | Web 数据源 `:528` | `MacWebSourcesView.swift:28`；列表详情、测试保存、删除；代码已查 | 网络测试期间取消无响应，见 M08 |
| W15 | iPhone 健康 `:537` | `MacHealthSummaryView.swift:24`；同步、指标、图表；代码已查，见 M03 | VoiceOver 每日数据、空快照/旧快照、减少动态效果 |
| W16 | 通知中心 `:546` | `MacNotificationCenterView.swift:47`；过滤、详情、清理/取消计划通知；代码已查 | 紧凑列表键盘选择，见 M01；已读状态变化时选择稳定 |
| W17 | 药物查询 `:555` | `MacMedicationSearchView.swift:22`；搜索、加载、空结果、标签信息；代码已查 | 搜索取消、查询与结果归属、FDA 来源措辞与小字号 |
| W18 | 模型测试 `:564` | `MacModelTestView.swift:132`；配置/输出双栏或分段，运行/停止；代码已查 | 停止后迟到结果、首 token 指标、配置对应当前所测模型 |
| W19 | 模型参数 `:573` | `MacModelParametersView.swift:356`；参数、校验、保存、放弃；代码已查 | 系统红色关闭按钮是否绕过未保存确认，见 M10 |
| W20 | 设置 `:581` | `MacSettingsView.swift` 实际位于 `NolaMacApp.swift:823`；七分类已逐项读；见 M02/M06/M07 | URL/token AX 标签，提交/失焦保存契约、所有分类标题 |
| W21 | 文件预览，附件/文件链接调用 | `MacQuickLookController.swift:19`；NSWindow + PDFKit/Quick Look；代码已查，见 M09 | 多文件切换标题、键盘前后翻页、不可读/受密码保护文件 |

### 工作台内页面与设置分区

| 页面或分区 | 入口/实现 | 已查内容与剩余风险 |
| --- | --- | --- |
| 启动加载/启动失败/不兼容数据库恢复 | `NolaMacApp.swift:2151` | 有详情、重试、归档恢复路径；启动慢时仅 spinner 标为 Nola，缺阶段/取消解释；实际迁移状态须测试 |
| 首次引导四步 | `MacOnboardingView.swift:47` | 步数、稍后、前后按钮、滚动、Reduce Motion 已查；未逐语言截图 |
| 聊天空态与快捷建议 | `MacConversationCanvasComponents.swift:284` | 非空建议可行动；不同执行环境不可用时说明须实测 |
| 会话搜索条 | `MacConversationCanvasComponents.swift:242` | 初始焦点、清除已查；关闭图标仅 help，按键返回与结果导航须补验收 |
| 会话侧栏/置顶/重命名 | `MacConversationSidebarView.swift:28` | Button 行与空态正确；删除无确认见 M12；重命名失败见 M12 |
| 输入框、附件、模型/运行位置/三档/权限菜单 | `MacComposerView.swift:80`、`MacChatWorkspaceView.swift:900` | TextKit hint 与 IME 已查；非标准 NSTextView AX 标签及焦点要实测；菜单密度、长模型名 |
| 思考过程/工具轨迹/任务卡/压缩源引用 | `MacThinkingProcessView.swift:92`、`MacTraceView.swift:32`、`MacMessageView.swift:705` | 思考标题已是28pt Button，不能再报旧18pt问题；内部 trace 仍见 M01；任务卡只静态核查，实际子任务状态需测试 |
| 内置权限卡/补充指令 | `MacHostPermissionView.swift:26` | 默认折叠参数、单次授权突出、文件夹范围解释已查；多按钮横排见 M11 |
| 模型追问卡 | `MacConversationCanvasComponents.swift:9` | 选择后仅颜色/描边变化，见 M14；提交期间状态需按真实回执验证 |
| 恢复提示/本地或远端运行恢复 | `MacConversationCanvas.swift`、`MacRunRecoveryPresentation.swift` | 可见入口已识别；错误→可恢复→继续→取消全部运行状态未实测 |
| Inspector 概览 | `MacChatInspectorView.swift:103` | 工作区、上下文/cache、来源、响应设置分层已查；窄宽度和统计AX读取需验收 |
| Inspector 数据源会话开关 | `MacInspectorDataSourcesView.swift:103` | 无标签开关见 M02；继承/禁用原因呈现须测 |
| Inspector 知识点 | `MacInspectorKnowledgeView.swift:114` | 编辑器/保存状态已查；保存失败/关闭重新打开要实测 |
| Inspector Wiki | `MacInspectorKnowledgeView.swift:139` | 独立 pane 与文本编辑，代码已查；长文编辑及恢复待测试 |
| Inspector Recall | `MacInspectorKnowledgeView.swift` | pane 和查询/来源入口已识别；检索结果定位全链路未实测 |
| Inspector 浏览器 | `MacInspectorBrowserView.swift:31` | Playwright/Chrome 两块、预览与切页已查；连接中断、展开后预览失效待测试 |
| Inspector 工具 | `MacInspectorActivityView.swift:13` | 状态由 executionLocation 推断 Enabled，见 M15 |
| 代码/文件 Inspector | `MacCodeReferencePreview.swift:492` | 加载/失败、复制、外部打开已查；9pt路径标签见 M00；二进制/PDF嵌入尺寸待测 |
| 数据中心 概览/数据源/记忆/使用记录 | `MacDataCenterView.swift:33` | 四个 section 与下级健康/Web路由已读；8pt契约标签见 M00 |
| 模型中心 本地 | `MacLocalModelManagerView.swift:54` | 可见库/下载/启动/删除/参数/存储设置；大模型启动失败不作为UI已实测 |
| 模型中心 API连接与模型 | `MacRemoteModelManagerView.swift:410` | 独立连接导航、反馈弹层、发现模型、保存确认已读；跨窗口关闭保护待验收 |
| 扩展目录与 Skill/MCP详情 | `MacExtensionsCenterView.swift:106/1218` | 开关/删除/编辑与放弃确认已读；跨目标关闭风险见 M10 |
| 设置 常规 | `NolaMacApp.swift:1483` | 登录启动切换失败回退与错误呈现已有实现 |
| 设置 Providers | `:1495` | 模型计数/默认模型/模型中心入口已查 |
| 设置 Agent | `:1527` | 默认记忆/检索、上下文限制、用户偏好；见 M06/M07 |
| 设置 Connections | `:1599` | 本机分享、远端连接、Keychain说明、删除确认；见 M02 |
| 设置 Data | `:1714` | 数据中心/数据管理目的地已查 |
| 设置 Diagnostics | `:1740` | 日志开关/等级与隐私说明已查 |
| 设置 Notifications | `:1771` | 系统状态、权限请求、事件开关、频率已查；授权回退需OS实测 |

## 模态界面与系统面板清单

以下逐项核对呈现入口和主要按钮。系统文件面板/TCC 弹窗的系统内容不由 Nola 设计；只审 Nola 的触发、说明、取消和结果处理。表中“待测”不是审查通过。

| 类型与任务 | 呈现入口 | 检查结果 |
| --- | --- | --- |
| 命令面板 sheet | `MacChatWorkspaceView.swift:452` | 搜索初焦、↑↓、Return、退出路径已查；多窗命令路由待测 |
| 会话模型覆盖 sheet | `MacConversationCanvas.swift:232` | 固定宽520、默认/取消/保存；输入非法仅红色见 M16；未保存退出保护不足 |
| 历史原文 sheet | `MacMessageView.swift:36` | `MacDeferredTextReader` 有取消快捷键、分页/重试；大原文性能待测 |
| Journal 全文 reader sheet | `MacJournalMessageTextView.swift:1150` | 独立 reader 入口与失败重试已查；全实现读页并发未在本轮运行验证 |
| 历史结构化详情 sheet | `MacHistoricalPartDetailsView.swift:17` | 关闭、part选择、失败重试、缺失/延迟状态已查；partID/字段名属于技术阅读器而非主会话文案 |
| 选择文本 sheet | `MacMessageView.swift:506` | Copy/Done、loading/retry；仅 Done 的关闭快捷键一致性待测 |
| 复制失败 alert | `MacMessageView.swift:456` | 中性标题+详情；是否能以内联重试替代属于设计优化 |
| 附件打开失败 alert | `MacMessageView.swift:498` | 有退出；未提供具体恢复动作 |
| 托管浏览器展开 sheet | `MacInspectorBrowserView.swift:16` | 最小900×640；窗口大小/浏览器内容消失后空面板待测，见 M11 |
| 添加API模型 sheet | `MacRemoteModelManagerView.swift:491` | 两页、默认用途/tool calls开启、校验；固定620×680见 M11；取消草稿保护待补 |
| API反馈 popover | `MacRemoteModelManagerView.swift:449` | 520×250详情；需要实际测长错误和键盘退出 |
| API切换目标未保存确认 | `MacRemoteModelManagerView.swift:496` | Save and Continue/Discard/Keep Editing，已有明确选择 |
| 删除API端点确认 | `MacRemoteModelManagerView.swift:520` | 明确“保存时移除模型及key”，取消存在 |
| API放弃修改确认 | `MacRemoteModelManagerView.swift:546` | 放弃+取消；恢复编辑状态代码已查 |
| API冲突重载确认 | `MacRemoteModelManagerView.swift:569` | 明确Reload and Discard；已有防误丢保护 |
| 本地模型参数 sheet | `MacLocalModelManagerView.swift:104` | interactiveDismissDisabled；由Done走确认，比独立参数窗口保护更强 |
| 内置本地模型删除确认 | `MacLocalModelManagerView.swift:115` | 说明删除下载文件、取消存在 |
| 动态本地模型删除确认 | `MacLocalModelManagerView.swift:129` | 删除失败显示错误，当前runtime target回退需实测 |
| 下载与存储 popover | `MacLocalModelManagerView.swift:208` | 固定宽370、源/存储卡；长路径和大字体待测 |
| 模型参数放弃/重载确认 | `MacModelParametersView.swift:399` | 按自定义Done触发；系统关窗见 M10 |
| 自动化编辑 sheet | `MacAutomationCenterView.swift:137` | Form、实时校验、保存错误、焦点、快捷键；取消直接丢草稿见 M10 |
| 自动化删除确认 | `MacAutomationCenterView.swift:146` | 说明保留已有历史、删除未来计划；运行中任务语义待测 |
| 活动清空确认 | `MacActivityCenterView.swift:147` | 说明只清列表，不删会话/任务日志 |
| 扩展删除确认 | `MacExtensionsCenterView.swift:132` | 取消与移除，并说明内置可恢复；远端项删除能力需按source验证 |
| Chrome管理 sheet | `MacExtensionsCenterView.swift:150` | Done、连接/安装操作与失败；退出快捷键、安装中关闭待测 |
| Skill编辑放弃确认 | `MacExtensionsCenterView.swift:1335` | 自定义返回保护；整窗/工作台导航离开见 M10 |
| 数据备份恢复预览 sheet | `MacDataManagementView.swift:158` | 摘要、取消、恢复；提交后关闭预览回主页面状态 |
| 清空应用数据确认 | `MacDataManagementView.swift:148` | 不可逆说明、建议先导出、取消存在 |
| 记忆编辑 sheet | `MacMemoryCenterView.swift:133` | 内容/摘要/作用域/类型/置信度；保存状态及错误见 M04 |
| 记忆删除确认 | `MacMemoryCenterView.swift:138` | 告知可能恢复上一版本；失败显示见 M04 |
| 记忆放弃确认 | `MacMemoryCenterView.swift:393` | 有discard/cancel；保存中取消竞态待修 |
| 提示词编辑 sheet | `MacPromptLibraryView.swift:64` | 校验/焦点/取消保护/保存已查 |
| 提示词删除确认 | `MacPromptLibraryView.swift:89` | 自定义模板不可撤销提示，默认模板删除后清空引用 |
| 提示词放弃确认 | `MacPromptLibraryView.swift:557` | 保留编辑/放弃，交互关闭禁用已查 |
| Web源编辑 sheet | `MacWebSourcesView.swift:82` | 测试并保存、状态/错误；测试中取消见 M08 |
| Web源删除确认 | `MacWebSourcesView.swift:97` | 告知移除缓存与运行历史，取消存在 |
| Web源放弃确认 | `MacWebSourcesView.swift:595` | 已有草稿保护；测试过程中被guard拦截 |
| Git提交确认 | `MacWorkspaceReviewView.swift:162` | 明确所有变更而非仅选中项，显示提交信息 |
| Git推送确认 | `MacWorkspaceReviewView.swift:172` | 当前分支/upstream说明；建议显示实际remote+branch以减少歧义 |
| 诊断清日志确认 | `MacDiagnosticsCenterView.swift:336` | 取消+清除，有失败状态回查 |
| 通知清历史确认 | `MacNotificationCenterView.swift:98` | 明确保留计划通知 |
| 单条通知删除/取消确认 | `MacNotificationCenterView.swift:113` | 两种动作不同标题与说明，保留 |
| 设置移除Host确认 | `NolaMacApp.swift:930` | destructive+cancel，范围说明已有 |
| 工作区选择 NSOpenPanel | `MacSecurityScopedBookmarkStore.swift:21` | 仅目录单选、不允许创建；runModal带来app级阻塞，见 M17。不能把用户主动选择目录误报为工具授权强弹窗口 |
| 输入附件 fileImporter | `MacComposerView.swift:244` | 系统选择入口；多选/提取失败必须运行测试 |
| 备份导入 fileImporter | `MacDataManagementView.swift:134` | 先预览后确认；cancel不执行恢复 |
| 提示词导入 fileImporter | `MacPromptLibraryView.swift:77` | security scope与错误处理存在；大文件在UI回调解码需测试 |
| 备份导出 NSSavePanel | `MacDataManagementView.swift:173` | begin异步回调，有exporting/错误状态 |
| 提示词导出 NSSavePanel | `MacPromptLibraryView.swift:328` | begin回调，有错误回传 |
| 日志导出 NSSavePanel | `MacDiagnosticsCenterView.swift:794` | runModal；跨窗口阻塞见 M17 |
| 文件不可读 NSAlert | `MacQuickLookController.swift:28` | 优先当前窗口sheet；仅“不可读”，无恢复入口 |
| OS通知/麦克风/联系人等系统授权 | `MacCapabilityService`、`MacPlatformDataService`、`MacRunNotificationService` | 系统UI不算Nola自绘窗口；本轮只检查可见触发入口，拒绝/受管设备/应用回前台完整矩阵待测 |

## Critical 必须修复

### M00 可见业务文字仍低于 Mac 推荐最小字号

- 证据：`MacPageChrome.swift:493`公共指标名9.5pt；`MacDataCenterView.swift:968`契约标题8pt、`:971`值9.5pt；`MacHealthSummaryView.swift:107`图表标题9.5pt、`:174`单位9.5pt；`MacMedicationSearchView.swift:163`段落标题9.5pt；`MacCodeReferencePreview.swift:537`父目录9pt；`MacMemoryCenterView.swift:209`来源/更新时间9.5pt。这里列的是文本，不把小尺寸SF Symbol本身误当字号违规。
- 原因：`accessibility.md › Vision`：“Use recommended defaults for custom type sizes.” 表中 macOS minimum 10pt；这些是识别状态和来源需要的实际信息。
- 修法：将公共 caption/metadata token统一至少10pt，常用状态11–12pt。不要对所有图标机械升到10pt；对可见文本做静态规则，白名单仅装饰图形。桌面长文另给缩放，不虚构Mac Dynamic Type支持。
- 验收：主窗、所有中心页、Inspector和各编辑页在系统缩放后，无8/9pt业务文案；明暗/高对比度截图测实际颜色后再给比值。

### M01 自绘可选行和轨迹只有鼠标手势 没有完整键盘动作

- 证据：`MacPromptLibraryView.swift:286`选择行用`onTapGesture`；`MacActivityCenterView.swift:224`同样；通知紧凑选择`MacNotificationCenterView.swift:535`同样。公共`MacSelectableListRowModifier`（`MacPageChrome.swift:345`）只画背景，无Button、focusable或AX action。工具轨迹`MacTraceView.swift:187`仅手势，`:194`加`.isButton`但没有实现键盘触发路径。
- 原因：`keyboards.md › Best practices`：“Support Full Keyboard Access when possible.” 视觉按钮trait不能代替原生按钮操作契约。
- 修法：列表用`List(selection:)`+tag并提供明确打开动作，或原生Button包裹内容；不要靠添加更多onTapGesture补键盘。轨迹展开用Button/DisclosureGroup，保留文件链接独立按钮，焦点与展开状态可读。
- 验收：不使用鼠标完成提示词选择、活动选择、紧凑通知详情、工具输出展开；Space/Return、上下键、VoiceOver激活与可见焦点一致。

### M02 无标签的数据开关与连接输入字段

- 证据：`MacInspectorDataSourcesView.swift:136`为`Toggle("", ...)`，附近标题是独立Text，没有accessibilityLabel；`NolaMacApp.swift:1652` URL TextField和`:1663`SecureField均为空标题，外部Text没有label关系。
- 原因：`accessibility.md › Vision`：“Describe your app’s interface and content for VoiceOver.” 权限相关开关尤其不能只报“开关，打开”。
- 修法：Toggle直接使用source title并labelsHidden；恢复默认按钮附带具体source名；连接字段以实际label初始化或明确accessibilityLabel，安全输入不暴露值。保留视觉布局，不要求重复显示文字。
- 验收：VoiceOver可区分每个数据源与全局继承、URL、access token；不依赖读前后元素猜测。

### M03 健康趋势的逐日数据被无障碍树主动丢弃

- 证据：`MacHealthSummaryView.swift:118` `.accessibilityElement(children: .ignore)`，accessibilityValue仅把七日steps求和；视图未提供逐日表格替代。
- 原因：`accessibility.md › Vision`：“Describe your app’s interface and content for VoiceOver.” 看图的人得到趋势，读屏的人只有总和，信息不等价。
- 修法：保留Charts每个BarMark日期和值的语义，或提供可访问数据表/图表描述；总和作为摘要，不替代各天。验证日期格式与单位。

### M14 模型追问选项只以颜色表示选中

- 证据：`MacConversationCanvasComponents.swift:40`按钮设置answer，`:58`选中差异仅背景与stroke，按钮未提供selected trait、checkmark或选择值。无自由输入时，读屏无法确认当前选项。
- 原因：`color.md › Inclusive color`：“Avoid relying solely on color”。
- 修法：使用radio Picker语义或增加checkmark及selected trait；提交前明确当前选择。不得把选择即提交，保留用户确认。

## High 交互与状态模型问题

### M04 记忆保存和删除失败在已有数据时不可见 且可重复提交

- 证据：`MacMemoryCenterView.swift:105`只有`errorMessage && records.isEmpty`显示错误；`:239/250`保存/删除catch仅设置errorMessage。编辑sheet不接收error或isSaving，`:383`保存只按内容是否为空禁用。
- 影响：用户看到无反应，可能重复创建或重复修订；失败并未让用户知道下一步。
- 修法：给异步编辑统一`idle/saving/failed/succeeded`状态；错误留在editor内，所有列表错误也应有非空列表上方notice；保存期间禁重复提交，关闭明确取消是否已提交。不以关闭sheet伪装成功。
- 依据：`layout.md › Visual hierarchy`：“Order content by relative importance.” 具体异步状态契约是本次工程设计判断。

### M06 上下文限制仅回车保存 缺少明确提交提示

- 证据：`NolaMacApp.swift:1555–1561`contextLimit仅onSubmit钳制并持久化，没有明确的回车提交提示。`:1592`自动保存文案位于独立的长期记忆Section，其userPreferences确有onChange保存，不能把这段文案解释成对上下文限制的承诺。问题是同页字段的提交方式不一致：上下文限制切换分类、点击外部窗口或直接关闭时，当前代码没有显式失焦持久化路径，实际事件行为仍需窗口测试。
- 修法：选一种清晰契约：提交/失焦校验并落盘（展示未保存、保存失败），或显式Apply。禁止默默钳制而不说明16,384–2,000,000范围。输入无效不能声称已保存。
- 依据：`settings.md › Best practices`：“Make settings available in ways people expect.” 保存契约为设计判断。

### M07 打开记忆中心的按钮目标错误

- 证据：`NolaMacApp.swift:1583`打开`data-center`，按钮`:1586`叫Open Memory Center；数据中心新建navigation默认`.overview`（`MacDataCenterView.swift:25`），没有选择memory。
- 修法：打开既有`memory-center`窗口，或携带明确数据中心memory路由，不能仅改文案掩盖目的地错误。增加路由测试。
- 依据：`layout.md › Visual hierarchy`关于结构表达关系；此处为代码确认的导航缺陷。

### M08 Web源测试时 取消按钮可点但没有作用

- 证据：`MacWebSourcesView.swift:545`取消按钮一直可用；`:639`requestCancel中`guard !isSaving else { return }`。异步测试Task没有保存可取消句柄，sheet同时interactiveDismissDisabled。
- 修法：让“取消测试”取消请求并使迟到响应无效；若进入不可取消的持久化提交阶段，将按钮明确禁用并解释，而不是显示可点击的假取消。以操作generation避免取消后旧成功关掉新编辑器。
- 依据：`sheets.md › Best practices`：“Provide an alternative to the Done button.” 取消状态机细节为设计判断。

### M10 各编辑器退出保护只覆盖部分路径

- 后续深查修正：只拦截工作台section Binding还不足够，侧栏原动作随后仍无条件新建/切换会话。现在常驻/紧凑侧栏、菜单新建/聚焦、命令面板鼠标与键盘选择共用`MacWorkspaceNavigationActions`，将目标页和模型副作用作为一次可取消意图提交；取消不改会话ID、草稿、数量。模型管理入口的内部选择同样延迟至确认；提交时目标会话已删除则不切页。命令面板先完成关闭再展示弃稿确认，不叠加modal。通知和“独立窗口打开”不销毁当前编辑器，保留独立开窗语义。新增真实workspace模型+隔离DB回归，非仅协调器状态测试。2026-10-09 21:35 统一Mac测试 **12/12通过**（退出7、窗口关闭2、删除3），0失败/跳过/运行时警告；重跑已消除新增trailing-closure弃用警告。结果：`Test-NolaMac-2026.10.09_9-34-47-+0800.xcresult`。

- 证据：自动化`MacAutomationCenterView.swift:833`Cancel直接回调，整个editor无hasUnsavedChanges/交互关闭保护；API新增`:2105`直接dismiss；会话参数`MacConversationModelSettingsView.swift:154`直接dismiss。独立参数窗口仅自定义Done走requestDismiss（`MacModelParametersView.swift:456`），未见NSWindow关闭拦截；Skill详情仅自定义返回触发确认。
- 修法：统一编辑会话契约，明确初始snapshot、dirty、save、discard、save failure；系统关窗、Escape、左栏换页、模型中心tab切换、sheet退出均走同一个决策。不强制每一次无修改关闭弹窗。
- 依据：`sheets.md › Best practices`退出选择；`windows.md › Best practices`遵循系统窗口行为。系统红色关闭按钮绕过路径是需实际验证的风险，不是本轮运行复现。

### M11 模态固定尺寸与授权横排没有匹配工作台的窄布局

- 证据：添加模型sheet `MacRemoteModelManagerView.swift:2129`固定620×680；浏览器展开`MacInspectorBrowserView.swift:353`最小900×640；权限卡`MacHostPermissionView.swift:128`用单个HStack承载指导、拒绝、会话授权和单次授权，没有ViewThatFits/纵向后备。
- 修法：sheet依据可用窗口高度设滚动主体、稳定头尾；浏览器反复操作宜可独立窗口并支持较小size。权限按钮在小宽度/德文长文案时转两行或纵向，绝不截断授权范围。
- 依据：`windows.md › Best practices`：“Make sure that your windows adapt fluidly to different sizes”；`layout.md › Adaptability`。具体溢出程度待最小工作台宽度截图验证。

### M12 删除会话连同沙盒文件没有确认或可撤销路径

- 证据：`MacConversationSidebarView.swift:273`Delete直接调用`deleteConversation`；`MacWorkspaceModel.swift:1687`移除会话并`:1704`删除会话workspace；`MacAppEnvironment.swift:123`是`removeItem`，不是Trash。当前可见UI未告知生成文件也会消失，没有Undo。repository删除错误又在`:1707`被try?忽略；重命名`:1640/1653`也吞持久化失败。
- 修法：优先恢复式删除/回收站，或确认中列明会话及生成文件；用户工作区不能随会话删。操作失败保留原列表/编辑状态并显示可重试错误。取消/确认不是Core授权替代；本项是用户主动删除的反馈和数据损失边界。
- 依据：`alerts.md › Best practices`对不可撤销破坏性操作要求warning；`alerts.md › Buttons`：“If there’s a destructive action, include a Cancel button”。

### M15 工具页把执行位置当成启用能力 容易误导权限状态

- 证据：`MacInspectorActivityView.swift:15–41`Files、Shell/Git、Computer use仅以`executionLocation`判断，`:117`显示Enabled/Disabled；同页Browser主要以Chrome扩展连接判断，未对应当前Managed Browser实际状态。
- 修法：用Core投影区分“提供此工具”“需授权”“可调用”“不可用”，不要用客户端枚举猜测权限。未取得具体授权应写“使用时申请”，真实拒绝/平台不可用应可查看原因。
- 依据：`privacy.md › Best practices`：“Be transparent about how your app collects and uses people’s data.” 这是数据契约与UI共同问题，不能仅改绿色图标。

### M16 会话参数的非法输入仅红色提示 保存禁用却不解释字段

- 证据：`MacConversationModelSettingsView.swift:188–205`解析失败只insert invalidNumericFields并改前景红色；保存`:175`禁用，顶部并无对应字段错误列表；Seed同样。
- 修法：字段下显示具体范围/格式错误并关联AX hint；表单顶部给首个错误跳转，明确覆盖与继承值。颜色只做补充，不能唯一表示错误。
- 依据：`color.md › Inclusive color`；严重程度High，若AX测试证实无法确定错误则提升Critical。

## Medium 优化项

### M05 记忆搜索无结果被显示为从未有记忆

`MacMemoryCenterView.swift:115`对filteredRecords.isEmpty一律展示No memories yet，并建议新建。区分原始空列表和过滤空结果，后者显示当前查询、清除搜索。依据`layout.md › Visual hierarchy`，具体文案为设计判断。

### M09 文件预览切换后窗口标题仍是第一份文件

`MacQuickLookController.swift:41`仅创建时设置window.title；Gallery`:73–80`只更新本地index。通过binding或回调同步窗口标题，前后按钮加“上一个文件/下一个文件”AX标签，增加标准键盘导航。依据`windows.md › Desktop (macOS)`窗口状态应可识别；文档标题行为是设计判断。

### M13 重要操作反馈定时消失 可能来不及读

`MacDataManagementView.swift:126`4秒删除status；健康`:264`5秒反馈清理。操作完成后持久保留最近结果（可手动关闭），进度/失败不依赖短toast；复制成功的短反馈可保留。依据`accessibility.md › Cognitive`：“Minimize use of time-boxed interface elements.”

### M17 应用级文件面板打断其他窗口

`MacSecurityScopedBookmarkStore.swift:32`、`MacDiagnosticsCenterView.swift:797`使用runModal；备份/提示词导出已经使用begin，契约不统一。将用户选择绑定到发起窗口的sheet并异步处理，其他会话窗口仍可工作。依据`sheets.md › Desktop (macOS)`：“Let people interact with other app windows without first dismissing a sheet.”

### M18 减少动态效果没有覆盖全部自绘内容

健康同步feedback关闭`MacHealthSummaryView.swift:58`直接withAnimation，页面没有Reduce Motion读取；侧栏search hover `MacConversationSidebarView.swift:118`与参数saved `MacModelParametersView.swift:445`直接动画。统一从共享token取得允许/关闭动画决策。减透明目前公共panel有处理，但药物页`.regularMaterial/.thinMaterial`需运行确认系统效果，不能仅凭材质调用断定违反设置。依据`accessibility.md › Cognitive`减少自动和重复动画。

### M19 信息层级与菜单入口不统一

`NolaMacApp.swift:699–738`Workspace菜单有八项管理目的地，药物查询、健康、模型测试、数据管理等仍主要靠其他页面入口；Git提交/推送没有本地菜单命令。建议共享Command定义驱动菜单/toolbar/命令面板，非聊天窗口也有有效FocusedScene命令。设置“Providers”仅摘要跳转可保留，但记忆同时独立窗口与数据tab应共享明确路由。依据`designing-for-macos.md › Best practices`：“Use the menu bar to give people easy access to all the commands”。不要给每个动作发明快捷键。

## 应保留的实现

- 思考过程主标题已是原生Button且最小28pt，显式展开选择覆盖自动折叠策略；不能重复修旧缺陷。
- 输入框用TextKit自身容器绘制hint，Return不抢中文输入法marked text；不透明背景遵循用户明确要求，不为玻璃效果改变这一点。
- 权限卡默认隐藏原始参数，保留完整查看；单次优先与文件夹精确范围说明，不扩大授权。
- API模型默认用途与tool calls已开启，端点切换提供保存/放弃/继续编辑；不要再引入强制能力确认。
- 数据管理恢复先展示摘要、通知删除区分计划取消、Web源保存前测试、模型测试可停止、提示词编辑保护，这些已有的正确模式应成为共享交互契约。
- 公共PageHeader已有ViewThatFits，多个详情已有compact后备；需要补一致性，不重建另一套无关联布局规则。

## 验收缺口与修复顺序

1. 先修M01/M02/M03/M14和M12：能否独立操作、理解状态、避免无提示数据丢失。
2. 统一异步编辑事务与导航退出（M04/M06/M08/M10），覆盖保存中关闭、失败重试、迟到回包、跨窗口修改。
3. 统一caption/metadata字号、表单错误、窄窗口按钮布局；保留大代码/表格宽度与正文阅读限宽分离。
4. 在每个上表窗口做实际验收：最小允许尺寸、1280×820、超宽/窄高；浅/深色、高对比度、减少透明度、减少动态效果；中文/英文/长德文；键盘、VoiceOver；空/加载/成功/失败/长内容/权限等待。
5. 聊天另录制完整链路：assistant shell→reasoning stream→tool→partial Markdown→跟随→完成→手动上滚→展开→恢复/重启。静态PNG无法证明“水面浮现”动画或结束空白已修。

`MacVisualSnapshotHarness.swift:11`只有17种Page，不涵盖全部窗口和弹层；不能以17张图代表以上全覆盖。本轮未执行任何窗口的视觉/VoiceOver测试，没有对真实数据库恢复、删除、授权或网络模型作交互验证。已枚举的低层reader、系统TCC和所有异步状态仍按上述缺口验收，不能写成“所有窗口均已通过”。

## 最新会话补充：底部遮挡与工具名称（2026-10-09）

- **High / 已改、待集中运行验收**：底部原先采用 `ZStack` 透明覆盖，再异步测量输入区高度补 transcript 留白；权限/恢复卡改变高度时，消息时间戳和操作区仍可能同时绘制。`MacConversationCanvas` 改为真实上下独立布局，底部使用语义不透明窗口背景与分隔线，移除测高补白状态；权限内容继续有滚动高度上限。依据 `layout.md › Guides and safe areas`：“Respecting the safe area is essential”，以及 `Visual hierarchy` 的关联分组原则；不新增主题。
- **High / 已改、待集中运行验收**：canonical 工具路径曾直接显示 `trace.title`，绕过已有本地化映射。统一标题投影覆盖工具行和思考分组；`write_file` 等内置名称使用已有本地化，第三方声明标题保留，capability ID、原始 journal 和权限协议不修改。
- 确定未提交的错误显示在独立可关闭面板，提示草稿保留并允许用户检查后重发，不显示用于“结果未知”的运行状态检查按钮。反馈就近可读、可选择复制，遵循 `feedback.md › Best practices`：显示命令无法执行的原因。
- 新增 canonical 标识本地化与第三方标题保留测试；现有紧凑窗口长权限测试增加 transcript 与底部区域不相交断言。集中验收最终通过 **44/44**（submission ticket 7、local terminal 16、process session 5、tool presentation 7、interaction boundary 9），0 failed/skipped/error/warning。首轮新增测试错误地要求内部 `tool_search` 出现在 canonical trace，按既有隐藏契约修正 fixture 后重跑全部 5 组通过；直接标题本地化断言仍保留。
- 最新 xcresult：`Test-NolaMac-2026.10.09_10-44-46-+0800.xcresult`；日志 `/tmp/nola-cache-diagnostic.DElgx5/mac-remediation-tests.log`。实际查看 `/tmp/nola-ui-remediation-permission-560.png`（560×420 点、1120×840 像素）：长权限内容在自身区域滚动裁剪，底部输入和配置仍完整可见，工具标题中文。极小剩余消息区域会裁剪可滚动的空会话欢迎标题；本图不代表正常长对话、所有窗口或所有辅助功能视觉验收。
