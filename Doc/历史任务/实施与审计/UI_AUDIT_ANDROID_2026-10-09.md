> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Android 全窗口 UI 审查

日期：2026-10-09。范围：Android 原生 Compose / Material 3 客户端，从 MainActivity、NavHost 和各页面局部导航状态，追踪页面、菜单、弹窗、底部面板与系统选择器入口。结论：**Critical issues**。主要问题不是某张卡片不够美观，而是导航、编辑事务、可访问性语义和容器适配没有统一契约。

## 已授权整改进度

后续用户授权修改后已实现下列一组整改；下文审查事实和原行号保留作为问题基线。**代码已改不等于通过编译或设备验收**，本分支未自行启动大型构建，由主代理在整组完成后统一验证。

| 问题 | 本组状态 |
| --- | --- |
| A01 | 历史菜单有朗读名；共享ToggleRow、参数和模型能力开关合并标签与toggle语义；工具/来源展开改标准TextButton。消息长按和其他零散开关、任务状态最终TalkBack树尚未全覆盖。 |
| A02 | 浅色Material主色改#0066CC、tertiary改#B33E22、error改#BA1A1A；新增实际主题配对对比度JVM测试。旧NolaColors静态token与透明合成/全窗口对比验收未完成。 |
| A03 | Chat和Home子页面新增系统返回处理；Home/Chat子页状态改saveable；数据页进入定时任务的创建动作接入聊天导航。未大改整个导航框架。 |
| A04 | 引导门控改Loading/null与true/false三态，未读完偏好不再先展示引导。冷启动仍待实测。 |
| A05 | 能力面板、Skill/MCP编辑、学习反馈、会话限制可滚动；反馈和定时编辑增加IME避让。字体200%与低高度待设备验证。 |
| A06 | 端点失败保留草稿；Skill/MCP/记忆/学习反馈/上下文返回有dirty确认；输入文本saveable；知识页明确返回前flush防抖。附件草稿和所有编辑器旋转/进程死亡恢复尚未完成，未将大型附件或密钥写入SavedState Bundle。 |
| A07 | 端点保存busy/error进入ViewModel状态，成功回执才关闭；失败保留输入且可重试；取消异常不伪装为保存失败。 |
| A08 | 新共享EditorSheetState统一Hidden拦截，应用于端点、Host、能力、学习反馈、定时编辑；参数原有完整保护保留。 |
| A09 | 本地模型新增删除确认；记忆/偏好新增确认并在等待期间检查目标仍与原值一致，变化后禁止删除。 |
| A10 | 新增自定义数据源的allowCloudContext默认关闭；既有数据源用户设置不改。 |
| A11 | 模型测试运行时提供停止按钮、允许取消退出，接入已有Job取消，不继续锁住用户。 |
| A12 | 跟随 effect 包含两类 permission request identity；新增列表外常驻待授权入口，显示实际请求数并只在点击时定位原卡片，polite live-region 通知状态。系统权限面板接管后排除不存在的本地卡片；补 4 项 JVM 投影回归与 2 项 Compose 语义/显式导航/读历史迟到授权回归。真实慢授权滚动及 TalkBack 设备验收仍待完成。 |
| A13 | 返回前flush知识/Wiki；未统一整页自动/手动保存或修改文档导入协议。 |
| A14 | canonical Reasoning复用ThinkingBlock本地化折叠；用户手动展开不被完成事件强制收起；个人智能分页有更多入口并修限宽顺序。真实流式状态和视觉待验。 |
| A15 | 自定义发送/附件按钮和配置pill升至48dp；共享toggle行最小48dp；引导下一步允许大字增高并支持系统返回上一步。其余触摸扩展测量未完成。 |
| A16 | 暂未改历史typed presentation格式化，避免在UI阶段猜测协议事实。 |
| A17 | 定时任务LLM摘要复用MarkdownRenderer；公式随字号/图表无障碍等仍待专项验收。 |
| A18 | 定时编辑增加 IME 避让及退出保护；日期/时间改 Material DatePicker/TimePicker（低高度或大字用 TimeInput），时区用可搜索列表与设备时区快捷项。各选择器取消不改原草稿、确认仅改草稿；更换时区明确保留当地日期/钟表时间。一次性任务严格解析并保存 offset：不存在时间禁保存、重复时间显式选第一次/第二次；已有 Z/offset 保留实际时刻。daily/weekly 仍是无日期 wall-time，说明既有调度器 DST 规则，不改核心。最终提交成功回执才关闭；失败保留草稿和稳定 ID、可重试，保存中防重入；区分未保存与已保存但调度失败。新增日期时区 9 项 JVM、保存状态 4 项 JVM 均通过；3 项 Compose 源码编译通过，未执行设备测试。Android 现有实体无 cron 编辑字段，不新增虚假支持；未知频率不转写、提示后仅允许用户显式选择支持的频率。 |

新增验证：`EditorDismissDecisionTest` 3例、`LightThemeContrastTest` 1例（4个颜色配对）；`ModelEditorRecoveryUiTest` 3例、`AccessibleToggleUiTest` 1例。建议统一命令：`cd android && ./gradlew :app:testDebugUnitTest --tests 'com.nola.ui.components.EditorDismissDecisionTest' --tests 'com.nola.ui.theme.LightThemeContrastTest' :app:compileDebugAndroidTestKotlin`。仪器测试真实执行需要可用设备，不把编译测试源码当执行通过。

初次审查阶段只新增本文档；上方记录的是后续授权实现。下表每项已检查入口与相应 UI 实现；不是设备截图验收，也不代表每个 ViewModel 分支、每条翻译和所有运行状态已验证。未访问用户数据库。所有窗口均仍需真机/模拟器视觉与 TalkBack 验收。

## 方法和适用规范

后续统一验证：指定 Gradle 命令退出码 0（`BUILD SUCCESSFUL in 9s`）；新增 JVM 回归 4/4 通过，零失败；instrumentation 测试源码编译成功（UP-TO-DATE），未运行设备测试。首次文件复制阻塞已解除，没有删除缓存。JUnit 结果时间戳为 `2026-10-09T13:06:16`。

A12 后续验证：`./gradlew :app:testDebugUnitTest --tests 'com.nola.ui.screen.chat.PendingPermissionNavigationTest' :app:compileDebugAndroidTestKotlin --console=plain` 退出码 0（`BUILD SUCCESSFUL in 24s`）；本轮新增 JVM 4/4 通过、零失败，新增 2 项 Compose 测试源码编译通过但未执行设备测试。英中复数资源 XML 校验通过。入口只导航到稳定请求 identity 的最新位置；请求已经清除或被替换时不执行过期导航，定位卡片也不会自动恢复滚动追随。

A18 成组验证：`./gradlew :app:testDebugUnitTest --tests 'com.nola.ui.screen.scheduled.ScheduleTimeInputTest' --tests 'com.nola.ui.screen.scheduled.ScheduledActionSaveTest' --tests 'com.nola.ui.screen.chat.PendingPermissionNavigationTest' :app:compileDebugAndroidTestKotlin --console=plain` 退出码 0（`BUILD SUCCESSFUL in 15s`），JVM 共 17/17 通过（本轮新增 13 项 + A12 回归 4 项），0 失败；Compose 测试源码编译通过。英中资源 XML 校验通过。未清缓存、未连接设备，不能据此宣称 Material 日历/时钟的实际小屏、200% 字号或 TalkBack 已验收。

使用 apple-design，完整读取其技能说明及 accessibility、layout、typography、color、designing-for-ios、modality、entering-data、privacy、loading、cross-platform。Android 仅应用通用可访问性、布局、反馈和隐私原则，不强套 iOS 导航、44pt 控件或玻璃材质。控件按 Material 的 48dp 目标评估；Compose 的自动触摸扩展可能使实际点击区域大于布局尺寸，因此尺寸不足分别标记代码风险和待测边界。

保存报告沿用仓库 Doc 文档，不创建外部 Page。文件路径以下均相对仓库；行号对应本次审查时源码。

## 页面和窗口清单

代码路径基准为 `android/app/src/main/java/com/nola/`。清单共 **52 个 UI 表面**；复用弹窗按组件计一次，系统窗口按调用入口计一次。`已查`表示检查入口及实现，`边界`表示只能检查系统入口契约，`专项待验`表示实现已查但结果必须设备确认。

| # | 页面或窗口 | 入口及实现 | 本轮结果 |
| --- | --- | --- | --- |
| 1 | 首次启动引导四步 | MainActivity.kt:31 → OnboardingScreen.kt:90；Value/Runtime/Sources/Ready | 已查；启动门控、返回、大字问题 A04/A15 |
| 2 | 五个主导航和宽屏侧导航 | ui/navigation/NolaScaffold.kt:69、89、189、363 | 已查；保留宽度自适应；窄宽/横屏大字待验 |
| 3 | 聊天主区、输入框、空态、错误态 | ChatScreen.kt:181、833、1142、1434、1675 | 已查；A03/A05/A06/A12 |
| 4 | 模型选择菜单 | ChatScreen.kt:509 | 已查；分组/选中有表现；长模型名与TalkBack待验 |
| 5 | 历史会话列表 | ConversationListSheet.kt:67 | 已查；A01/A03；无搜索，历史量大需补 |
| 6 | 历史会话更多菜单 | ConversationListSheet.kt:226、235 | 已查；A01 |
| 7 | 删除会话确认 | ConversationListSheet.kt:82 | 已查；有取消和对象名；删除失败反馈专项待验 |
| 8 | 会话概览 | ChatContextSheet.kt:144、399、615 | 已查；A03/A06/A13 |
| 9 | 会话知识页与记忆搜索 | ChatContextSheet.kt:542、631 | 已查；A03/A06/A13 |
| 10 | 会话工具、数据源开关 | ChatContextSheet.kt:774、1045、1083 | 已查；开关标签、覆盖/默认状态TalkBack待验 |
| 11 | 会话 Skills/MCP/文档库 | ChatContextSheet.kt:144（Skills 模式） | 已查；导入/编辑流程 A06/A13 |
| 12 | 会话 Skill 编辑对话框 | ChatSkillEditorDialog.kt:18 | 已查；A06；保存期间有忙碌保护 |
| 13 | 会话 Skill 删除确认 | ChatContextSheet.kt:210 | 已查；有对象名/取消 |
| 14 | 会话更多设置 | ChatMoreSettingsSheet.kt:144 | 已查；A03/A01；权限三档文本省略待验 |
| 15 | 会话 token 限制对话框 | ChatMoreSettingsSheet.kt:291 | 已查；不可滚动 Column；键盘/大字专项待验 |
| 16 | 忘记配对确认 | ChatMoreSettingsSheet.kt:457 | 已查；有取消与说明 |
| 17 | 模型参数面板（两入口复用） | ModelParametersSheet.kt:46 | 已查；已有事务基线/dirty guard/保存错误；开关 A01 |
| 18 | 参数与连接放弃修改确认 | ModelParametersSheet.kt:226 | 已查；不同调用点隐藏行为不一致 A08 |
| 19 | 消息长按操作面板 | ChatScreen.kt:2419 | 已查；A01；窄高和大字布局待验 |
| 20 | 教会 Nola 反馈面板 | ChatScreen.kt:2516 | 已查；A05/A06/A01 |
| 21 | 个人智能详情 | PersonalIntelligenceDialog.kt:76 | 已查；A14；24 条记忆/10 条策略截断未给更多入口 |
| 22 | 清空个人智能确认 | PersonalIntelligenceDialog.kt:229 | 已查；清空同步操作反馈待验 |
| 23 | 历史部分详情 | HistoricalPartDetail.kt:68 | 已查；A16；有加载/重试/关闭 |
| 24 | 本地权限请求卡与追加指令 | ChatScreen.kt:1108 → CommonComponents.kt:838 | 已查；保留单次优先、48dp纵向按钮；A12 |
| 25 | Host/设备桥权限请求卡 | ChatScreen.kt:1077 → CommonComponents.kt:838 | 已查；有执行位置/数据/期限；A12 |
| 26 | 工具、来源、附件详情卡 | CanonicalPartCards.kt:35、87、133；CommonComponents.kt:工具卡 | 已查；A01/A05/A16 |
| 27 | Skill 与子任务展开卡/指导/取消 | ChatScreen.kt:2118、2249 | 已查；A01；错误结果详情专项待验 |
| 28 | Markdown / 代码 / 表格 / Mermaid | MarkdownRenderer.kt:112、222、373、610 | 已查；A14/A17；渲染/大字/图表无障碍专项待验 |
| 29 | 数据工作区概览 | DataWorkspaceScreen.kt:101 → HomeScreen.kt:85、97 | 已查；A03；加载/健康授权/数据空态分支存在 |
| 30 | 数据源工作区 | DataWorkspaceScreen.kt:218、237 | 已查；有刷新/错误；48dp、状态朗读待验 |
| 31 | 自定义数据源编辑 | DataWorkspaceScreen.kt:856、935 | 已查；有滚动/校验；A01/A10 |
| 32 | 自定义数据源删除确认 | DataWorkspaceScreen.kt:884 | 已查；有取消/对象上下文 |
| 33 | 自定义数据源放弃确认 | DataWorkspaceScreen.kt:909 | 已查；保留草稿/确认设计 |
| 34 | 智能页记忆/偏好/知识概览 | IntelligenceScreen.kt:主组件、93、204 | 已查；有加载错误重试；A06/A14 |
| 35 | 智能页 Skill/MCP 能力库 | IntelligenceScreen.kt:724、766、808 | 已查；角色权限/网格已有区分；大字待验 |
| 36 | 记忆/偏好编辑 | IntelligenceScreen.kt:1152 | 已查；A06 |
| 37 | 记忆/偏好更多菜单 | IntelligenceScreen.kt:682 | 已查；直接删除无撤销 A09 |
| 38 | Skill 编辑/只读详情 | IntelligenceScreen.kt:1008 | 已查；A05/A06 |
| 39 | MCP 编辑/只读详情 | IntelligenceScreen.kt:1062 | 已查；URL 校验存在；A06/A01 |
| 40 | 能力删除确认及保存错误提示 | IntelligenceScreen.kt:372、399 | 已查；相同页面多个独立 modal 状态，需单一仲裁 |
| 41 | 模型中心主列表/下载/连接状态 | ModelsScreen.kt:124 | 已查；A07/A09/A11；本地下载完成/失败待验 |
| 42 | API 端点编辑 | ModelsScreen.kt:1068 | 已查；A07/A08；URL校验/密码遮罩保留 |
| 43 | Web Host 连接设置 | ModelsScreen.kt:1173 | 已查；A08；有错误/验证状态；运行中可编辑字段待验 |
| 44 | 模型执行能力 | ModelsScreen.kt:476 | 已查；A05/A08/A01 |
| 45 | 模型推理测试 | ModelsScreen.kt:879 | 已查；A11；已有流式输出和耗时 |
| 46 | 设置首页/数据管理/通知入口 | SettingsScreen.kt:89、119 | 已查；有BackHandler；清理/导出状态保留 |
| 47 | 清理全部数据与导出确认 | SettingsScreen.kt:156、197 | 已查；有确认/忙碌；成功失败提示专项待验 |
| 48 | 通知中心/筛选/详情 | NotificationCenterScreen.kt:85；NotificationDetailScreen.kt:63 | 已查；有BackHandler/重试/详情；保留 |
| 49 | 定时任务列表/菜单/删除确认 | ScheduledActionsScreen.kt:91、135、534、596 | 已查；筛选/确认存在；来自数据页创建入口 A03 |
| 50 | 定时任务编辑与放弃确认 | ScheduledActionsScreen.kt:1127、1166 | 已查；A08/A18 |
| 51 | 定时执行结果详情 | ScheduledActionsScreen.kt:846 | 已查；可滚动、失败说明、技术详情；Markdown摘要 A17 |
| 52 | 系统权限/文档/照片/健康/分享/外部打开窗口 | ChatScreen.kt:235–307、1889、2397；SettingsScreen.kt:217；HomeScreen.kt:112 | 边界；Android系统窗口不可从Compose证明，需设备走完取消/拒绝/无处理器流程 |

## Critical

### A01 控件语义没有贯穿到可访问性树

- **确定证据**：`ConversationListSheet.kt:226–233` 的 IconButton 唯一子 Icon `contentDescription = null`，且没有父标签；“更多”按钮无法表达用途。`ChatMoreSettingsSheet.kt:867`、`ModelParametersSheet.kt:159`、`ModelsScreen.kt:507` 等布局把 Text 和 Switch 放在普通 Row，开关没有关联的语义标签，朗读可能仅剩开/关。`CanonicalPartCards.kt:64、106、135` 用裸 clickable Text 承担展开和打开动作，无按钮角色/展开状态。`ChatScreen.kt:1843` 整条消息只有长按进入操作，onClick 为空，也没有可理解的 longClickLabel。
- **依据**：`accessibility.md › Vision`：“Describe your app’s interface and content for VoiceOver.” 在此对应 TalkBack，而非要求 Apple API。
- **修法**：更多按钮本地化标签包含会话名；开关 Row 使用 `toggleable(role = Role.Switch)`，子 Switch 回调设 null；展开使用 Button/语义 `stateDescription`，提供可访问性 custom action 或可见更多入口。加入 TalkBack 节点断言，不以 testTag 替代可访问性标签。

### A02 默认浅色主题存在可计算的文字对比不足

- **确定证据**：`ui/theme/Theme.kt:28–45`，primary `#0A84FF` 与 onPrimary 白色、surface 白色；实算 WCAG sRGB 相对亮度对比 **3.65:1**。普通浅色主按钮白字及 primary 小号文字不满足 4.5:1。error `#E6474D` 在白色上 **3.91:1**，影响表单错误。tertiary `#FF7A59` / 白色 **2.57:1**（仅指出 token 组合，不能断言每个页面都用其作文字）。`DesignSystem.kt:67` 另有硬编码 NolaColors，与 MaterialTheme 双轨。
- **依据**：`accessibility.md › Vision`：“Strive to meet color contrast minimum standards.”
- **修法**：统一角色 token 和 on-color 配对，为小字采用更深主色/错误色；分别验算白底、浅色容器、暗色容器及透明合成后的最终颜色。不要简单把所有蓝色换成一个值；装饰图标与文字可用不同强调层级。

## High

### A03 子页面不在统一返回栈内

`ChatScreen.kt:190–198、310–433` 用一组 remember 布尔及提前 return 显示会话设置/上下文/历史，文件内没有 BackHandler；`HomeScreen.kt:85–94` 同样用 activeScreen 替换详情而未注册 BackHandler。系统返回不能按可见层级返回上一页，可能退回主导航前页或退出；设置页有 BackHandler，体验不一致。且 `HomeScreen.kt:90` 没传 ScheduledActionsScreen 的 `onCreateInChat`，其默认空回调（`ScheduledActionsScreen.kt:93`）使数据页进入任务后的创建按钮无效。

修法：子页成为显式 Navigation 路由，统一返回和 state restoration；短期至少集中 BackHandler。让两个定时任务入口复用同一真实创建回调。依据 `accessibility.md › Cognitive`：“Keep actions simple and intuitive.”

### A04 冷启动暂用 false 导致旧用户进入引导的短暂窗口

`MainActivity.kt:31–37` 首次收集 `hasCompletedOnboarding` 使用 initial=false，读取尚未完成时即组合 OnboardingScreen，没有独立 loading/splash 门控。**代码路径风险，尚未设备复现闪烁时长**。修法：启动状态三态 Loading/NeedsOnboarding/Ready，偏好读完再显示正确页面；不能让存储IO时序决定首次引导是否闪现。依据 `loading.md › Best practices`；展示轻量启动占位而非错误页面。

### A05 大字和键盘下的面板存在不可达内容风险

`ChatScreen.kt:2526–2650` 学习反馈整块 Column 不可滚动，文本输入后仍有开关和保存；`ModelsScreen.kt:492–530` 能力面板同样无滚动，已有标题/帮助/3开关/3单选/按钮；`IntelligenceScreen.kt:1028–1045` Skill 编辑要求 minLines=10、maxLines=18，但外层 Column 无滚动；`ChatMoreSettingsSheet.kt:295` 限制对话框也无滚动。**未测具体裁切像素**，但容器不能为大字/低高度提供内容滚动是代码事实。

修法：统一可滚动编辑容器、IME inset、可见保存/取消区域；先测320dp宽、横屏、200%字体与键盘同时出现。依据 `layout.md › Adaptability`：“Be prepared for text-size changes.”

### A06 草稿生命周期不稳定且取消没有一致保护

`ChatScreen.kt:188、196` 输入文本/附件只用 remember，Activity重建可能丢失；`IntelligenceScreen.kt:1018、1083、1163`、`ChatSkillEditorDialog.kt:33` 与 `ChatScreen.kt:2526` 允许直接 dismiss，未检查已编辑内容。上下文 `ChatContextSheet.kt:182–186` 草稿以外部值为 remember key，刷新会重建；知识编辑 `:239–245` 700ms防抖副作用在离页时会取消，缺少离页 flush。修法：ViewModel/SavedStateHandle 持有分实体草稿，保存事务三态；统一 dirty guard，自动保存需完成后确认而非计时假定。依据 `modality.md › Best practices`：“help people avoid data loss”。

### A07 API 保存提前关闭编辑器，失败后不能原地恢复

`ModelsScreen.kt:148–153` 调用 viewModel.saveApiEndpoint 后立即隐藏；`ModelsViewModel.kt:379–405` 实际在 viewModelScope.launch 异步保存，失败只改主页面状态，不更新配置值。`ModelsScreen.kt:1076` 起编辑草稿仅用 remember，sheet 移出组合后草稿销毁，再打开会加载旧配置，因此失败确实丢失本次输入，也无保存忙碌边界。修法：与 ModelParametersSheet 同样返回成功/失败，成功才关闭，失败原位保留输入并展示可修复原因。不要延时关闭模拟成功。

### A08 大部分底部面板只在 onDismissRequest 中检查 dirty

`ModelsScreen.kt:492、1081、1186`、`ScheduledActionsScreen.kt:1191` 缺少 `confirmValueChange` 隐藏拦截；对比 `ModelParametersSheet.kt:97–105` 已显式阻止 dirty 时进入 Hidden。前者可能在滑动收起后才弹确认，选择继续编辑时原面板是否复原没有统一保障。**手势后的具体展示需要设备验证**。修法：复用同一 SheetState/草稿控制器，对返回、遮罩、拖动三个路径统一处理；保存时不允许状态悄悄进入 Hidden。

### A09 本地模型删除和记忆删除缺少恢复保护

`ModelsScreen.kt:798–809` 删除图标直接 `onDeleteLocal()` → `ModelsViewModel.kt:460` 删除文件，无确认/撤销。`IntelligenceScreen.kt:703–708` 记忆/偏好菜单直接删除，也无撤销。模型可重下载不等于操作无成本，可能数GB。修法：模型确认显示名称/占用/需重新下载；小型记忆可做可撤销删除。权限层是否授权与用户是否误触是两件事。依据 `modality.md › Best practices` 的数据丢失保护；具体确认内容为设计判断。

### A10 数据源云端上下文默认勾选与本地优先定位冲突

`DataWorkspaceScreen.kt:865、872` 新数据源 `allowCloudContext ?: true`，开关位于策略之后 `:992`。这不证明任何数据已泄露，但新增敏感资料时用户尚未主动选择就具有云端可用设置，和“敏感权限默认关闭、需要时申请”预期不一致。修法：新增源明确数据去向，云端默认不启用，按实际调用申请；保持普通模型工具能力和隐私授权两个不同层次，不重新引入逐模型能力开关地狱。依据 `privacy.md › Best practices`：“Be transparent about how your app collects and uses people’s data.”

### A11 推理测试有取消能力，但 UI 不给用户出口

`ModelsScreen.kt:891` running 时忽略 dismiss，按钮 `:917` 只可开始不可停止；`ModelsViewModel.kt:304–308` closeModelTest 实际已取消 job。用户遇到慢首token或无终止流无法主动停止。修法：运行时显示“停止测试”，关闭允许取消该测试；普通模型选择和其余操作不必锁死。依据 `loading.md › Best practices`：“Let people do other things ... while they wait”。

### A12 新权限请求不在滚动跟随的触发键里

`ChatScreen.kt:929` 跟随 LaunchedEffect 的键包含文本、thinking、toolEvents和isGenerating，但不含两个 permission request identity；权限卡到 `:1075、1104` 才作为新 LazyColumn item 插入，timelineMessages.size 不一定变化。特别是上一轮 bounded follow 已结束、用户停在旧内容时，新授权可能出现在屏幕外且无固定待处理入口。**这是条件性遗漏，不宣称每次授权都不可见**。修法：统一 viewport事件驱动器识别待处理授权，跟随开启时带到卡片，读历史时显示固定待处理按钮而非强拉滚动；加TalkBack polite状态提示。

## Medium

### A13 上下文页面混合自动保存、手动保存与全局编辑

`ChatContextSheet.kt:239` 自动保存知识/Wiki，`:615–704` 会话记忆与全局记忆/偏好分区，`:706–742` 手动保存按钮；同一页的编辑反馈不一致。修法：每块明确当前会话/所有会话、自动保存/保存按钮、dirty/saving/failed；不要用2.6秒后消失通知作为唯一成功证据。文件导入 `:262–275` 直接 readBytes/UTF-8 处理，真实常用文件支持/大文件主线程耗时需要另查，不应在UI宣称已成功预览PDF。

### A14 新旧展示链路的思考、动效和限宽不一致

CommonComponents.kt:215 的 ThinkingBlock 有本地化、48dp整行、减弱动态；CanonicalPartCards.kt:39 的 Reasoning 只输出 `part.title` 和摘要，既无同一折叠组件，也不在此处映射标题。需要验证服务端标题是否已本地化，不能假定UI已统一。CommonComponents.kt:224 在 finished 后强制 expanded=false，会覆盖用户主动展开阅读。PersonalIntelligenceDialog.kt:175 使用 take(24) 且未给更多入口；其 modifier 先 fillMaxWidth 再 widthIn(max=720)，应核实大屏约束是否失效。修法：统一 reasoning UI契约、用户展开优先；列表显示完整或明确分页。动效以系统设置验收，不因个别文件没调用辅助函数就断言系统禁用动画失效。

### A15 48dp 与字体缩放整改没有覆盖全部自定义控件

ChatScreen.kt:1662 自定义发送/附件Box size=44.dp，`:1410` 控制pill min40dp；DataWorkspaceScreen.kt:203 分段文字只有9dp纵向padding，无48dp最小区域；OnboardingScreen.kt:204 下一步固定height50dp而非heightIn。这些不是前轮修过的48dp附件删除/思考标题。修法：自定义控件建立min48dp布局及Role.Button，避免相邻触摸扩展重叠；按钮用heightIn让大字增高。实际扩展后的触摸矩形和切换开关最小尺寸需 instrumentation 验证，不只grep数字。

### A16 历史与工具详情直接暴露内部字段

HistoricalPartDetail.kt:102–111 直接显示 statusLabel、字段key、JSON toString；CanonicalPartCards.kt:70–79 展示毫秒时间戳、raw permission domain/decision、模型ID。不要把协议名误认成必须翻译，但可读日期、常见状态、字段名应本地化。修法：typed presentation formatter，摘要优先，完整原始JSON独立可复制详情；未知字段保留可检查，不删事实。

### A17 富文本体验存在渲染路径差异

MarkdownRenderer.kt:373 用Markwon TextView，:610 流式分块，:222 Mermaid WebView；需要同一fixture跨路径验收中文换行/表格横滚/代码复制/链接/数学公式。JLatexMathPlugin.create(16f)（:432）为固定渲染尺寸，缩放是否跟随正文需验证。ScheduledActionsScreen.kt:920 的LLM摘要用Text而非Markdown，有语法时将原样显示。修法：共享语义内容策略，对明确Markdown的摘要复用渲染器；Mermaid给可读源代码/图表说明和打开全屏，不只依赖SVG可视图。

### A18 定时任务仍要求手输日期和时区

ScheduledActionsScreen.kt:1260–1304 文本输入日期/时间和时区。校验已存在，但常用操作仍要用户记住格式及IANA ID；:1192 保存键也在滚动尾部。修法：Material DatePicker/TimePicker、可搜索时区选择，保留高级文本编辑；保存固定在安全/键盘区域内。依据 `entering-data.md › Best practices`：“offer choices instead of requiring text entry”。

## 应保留的设计

- Material 3体系、系统暗色跟随、sp文本、宽屏导航断点和多数标准IconButton，不换成仿iOS全局玻璃。
- 权限卡明确位置、数据边界、有效期，单次优先，拒绝和会话授权不隐藏。
- 参数编辑已有事务基线、saving状态、dirty隐藏拦截、刷新冲突和liveRegion错误，值得成为共享编辑器规范。
- 通知/任务已有筛选、空态、失败说明，通知详情可选技术数据；不要把原本清晰的一行状态改成无收益的折叠。
- 减弱动态、流式稳定块、末尾滚动锚点已有基础，修事件与用户意图，不重写全部动画。

## 下一轮验收和覆盖缺口

必须以52项清单逐一执行：320/360dp手机、横屏低高度、680/1180dp断点两侧、字体100/130/200%、浅暗色、中文/英文/德语长文、TalkBack与键盘、系统返回/遮罩/拖动取消、加载/离线/保存失败/空结果。重点加冷启动已完成引导、旋转保留草稿、正在授权时离页返回、结束流式Markdown高度变化、慢模型测试取消。

系统窗口（权限、DocumentsUI、照片、健康、分享、外部PDF应用）的真实布局、拒绝后恢复和活动重建尚未实测；所有Compose窗口的截图、触摸扩展边界、TalkBack最终树、折叠屏IME/返回行为仍未验收。web/CLI和Apple客户端不在本分报告范围。未宣称所有组件逐行审完或视觉符合要求；这是窗口级入口和主要交互代码清单及已证实/有条件的问题集。
