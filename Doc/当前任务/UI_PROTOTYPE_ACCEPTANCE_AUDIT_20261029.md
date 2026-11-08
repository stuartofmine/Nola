# 当前原型完整需求验收审计（2026-10-29）

审计基线：`docs/design/nola-ui-prototype/docs/UI-INTERACTION.md`、`DESIGN-SPEC.md`、原型根目录 `FEATURE-COVERAGE.csv`。后者不是在 docs 子目录。这里是当前任务的独立差距账，不替代实施总账，也不把测试全绿改写成“原型全部完成”。本次只读审计没有启动编译或测试。

范围：当前生产 Web/Svelte、iOS/SwiftUI、macOS/SwiftUI/AppKit、Android/Compose；CLI 单列适用项。审计以当前 caller、Core 端口和已有实际结果为依据。原型本身说明其回复、授权、版本、审阅和调度是本地模拟，因此模拟状态测试不能证明生产具备相应业务。

状态：**接线**＝发现真实生产入口及现协议；**部分**＝只有需求子集；**业务缺口**＝缺共享事实/命令，不能客户端模拟补齐；**待证**＝源码存在，但未有该项完整实际验收。接线和编译通过均不等于实际验收通过。

## 1. 必须继续处理的差距

| ID | 原型要求 | 当前事实与定位 | 类型及下一步 |
| --- | --- | --- | --- |
| A01 | 单个失败专家可重试，保留原身份/颜色 | Web `Chat.svelte` 仅对 selected/executing 暴露 `resumeCompleteExpertMessage`；Mac `MacAgentRoomsView` 的 retry 针对 pending 投递失败；各端完整 manifest 恢复解决的是未准入/原投递，不是 sealed failed 后的新执行尝试。 | **共享业务缺口**。需要 Core 显式新尝试身份、原完整定义/模型见证、原 parent 关联与失败 entry 精确归属。不能把原 intent 幂等 replay 宣称成新失败重试。 |
| A02 | 审阅：hunk、发现严重度、解决/重开、请求修改/通过，重要未解决项阻止通过 | Web `pages/WorkspaceReview.svelte` 实际显示文件状态及 staged/unstaged diff；Mac `MacInspectorWorkTools.swift`/`MacWorkspaceReviewView.swift` 有真实 Git 文件/diff/提交推送能力。没有看到统一 finding 生命周期和审阅决定门控。 | **部分＋共享业务缺口**。保留真实 Git 操作；另设计 finding/review decision/验收关联，不用 UI 的“通过”按钮伪造。 |
| A03 | 产物：类型、来源、保留版本、编辑成新版、版本比较、错误重试 | Web `ChatArtifacts.svelte` 汇集已加载消息附件及工具 artifacts；Mac `MacInspectorArtifactsView` 目前只遍历已加载 assistant attachmentURLs；iOS resources 是上传内容列表。没有共享产物版本编辑/比较闭环。 | **部分＋共享业务缺口**。定义产物身份/版本及 host/source 后再接编辑。Mac 的 tool-only artifact 是否进入 attachmentURLs 仍需实际覆盖，不能假定与 Web 等价。 |
| A04 | 目标：范围、优先级、里程碑、验收条件与授权/产物/审阅/人工确认映射 | 现 `ChatActivityGoal`、`MacInspectorProgressView`、`IOSConversationInspector`、Android `ConversationProgressPanel` 接真实 goal objective/requirements/work/control。不能由这些字段推出原型所有优先级/里程碑/跨对象验收关系。 | **部分＋共享业务缺口**。复用现 Goal 控制；新增关系须进入共享协议。 |
| A05 | 移动端可发现当前工作的概览/审阅/产物/进度/目标及更多 | iOS `IOSConversationWorkTools` 已提供 progress/goals/resources/context/experts/execution 六个真实入口，已有逐项进入并保草稿证据。Android 当前 `ConversationInspector` 仍是 Progress/Resources/Settings，其他工具分散在二级入口。 | **部分平台接线已完成，Android 组织仍待补**。完整审阅/产物业务仍依赖 A02/A03，不把 resources 当产物版本闭环。 |
| A06 | 专家可自定义并保存，模型/定义配置与快照一致 | 四端当前专家编辑已接模型选择/继承、模板可读目录及 Core 同事务定义捕获；Mac managed 模板实际发送与变更后原定义重发已过，Web 双专家真实原快照重发已过。iOS 模板服务观察修复后，实际创建、目录与选择、两 child Begin 均已通过旧失败点，但双专家长任务仍有一个超时；Android managed 模板实际案发现一次本地 UI 工具审批路径未处理，正在补真实操作。 | **主要 caller/共享授权已接，真实全链仍在收口**。sparse 数值 override 的 native 执行缺口仍须关闭，不能以捕获授权测试代替有效参数验证。 |
| A07 | 专家启停/归档与恢复可达 | iOS、Mac、Web 与 Android `ExpertManagementPanel` 当前均有 availability 操作；Android 用 configure_agent 的 exact revision CAS，含归档记录编辑/恢复入口。既有实际创建/归档/恢复证据保留，新 JNI 回归还在执行。 | **caller 缺口已补，当前实际回归待完成**；不再把旧“Android 没控件”当当前事实，也不以 picker 隐藏替代归档业务。 |
| A08 | 每会话详情阅读位置恢复 | Mac 文本详情保留 bounds；当前 `MacDetailScrollPositions.PDFViewport` 已按 owner/object 保存 PDF page/point/scale/autoScales，生产 PDFKit 接线及异步 token/新 owner 回归已过。 | **PDF/文本子集已补**。通用 QuickLook 阅读位置仍未取得能力/实测证明；不能从 PDFKit 推断所有文件类型已完成。 |
| A09 | 手机 Today 助理整理的今日事项、审阅/备注/稍后/已看、等待/接下来/已整理 | 各端 Today 现真实共享 `load_today_facts`，展示 needs_response、queued_instances、recent；iOS另有真实通知已读。没有自动整理日报、今日优先级、通用事项审阅/稍后/备注协议。recent 不是已完成成果。 | **拟议业务未实现**，不是“已有 Today 页面即完成”。必须保留事实语义，不从时间/最近会话虚构总结。 |
| A10 | 保存/取消清晰，管理编辑状态不会跨 owner 串写 | Web 现按 owner/标签挂载；Mac 模态管理编辑；手机管理与 inspector 独立导航。模型/资料/计划等全部二级编辑器仍缺一份逐操作 owner＋退出/取消/冲突实际验证。 | **待证**。已测会话草稿及某专家表单不能替代所有管理表单。原型不要求任意未保存标签编辑器，但要求现有表单取消/冲突语义。 |
| A11 | 字号/行距达到统一设计规范 | 当前元信息字号与专家宽列身份已修；Mac 明暗实际截图及完整姓名 AX 回归已有证据，Android 当前生产专家气泡矩阵已接。原 9.5 时间/48px 姓名列为已修阶段记录。 | **部分视觉差距已关闭，完整矩阵待证**。大字体、长中英文名、RTL、键盘焦点与全部 320px 场景仍未逐端覆盖，不从少量暗色截图推断全局一致。 |
| A12 | 错误、停止、补输入、拒绝与允许均有独立完整闭环 | 本轮 Core exact owner/恢复事务测试及部分真实停止/工具调用已覆盖；各端“真实拒绝→保留草稿→重新发起→精确审批→恢复”与未知投递/远程故障的完整 GUI 操作证据不齐。 | **待证**。协议通过不替代真实点击；不要为验收制造伪审批或丢弃未确认 Run。 |

## 2. 原型能力矩阵逐项对照

| FEATURE-COVERAGE 项 | Web | iOS | macOS | Android | 当前验收结论 |
| --- | --- | --- | --- | --- | --- |
| 会话知识点 | `ChatKnowledge.svelte` 编辑/冲突 | `ChatContextSheet` | `MacInspectorKnowledgeView` | `ChatContextScreen` | 接线；各端实际冲突＋保留草稿尚待证。 |
| 独立 Wiki | 会话资源/Wiki | 上下文页独立Wiki | 工作上下文/Wiki | 上下文知识页 | 接线；独立对象与保存流程不能与压缩摘要混同。全端编辑实测待证。 |
| 压缩摘要与历史召回 | timeline/回忆 | 上下文/历史读取 | `MacConversationRecall`/历史 reader | context/history projector | 接线；长历史＋锚点＋来源读取未在本批四端逐项真实验收。 |
| 个人长期记忆 | 智能/记忆管理页 | `PersonalIntelligenceDashboardView` | `MacPersonalIntelligenceView` | `IntelligenceScreen` | 有真实分类/依据/置信度读取；不能把能力条目删除当成每条长期记忆修正/确认删除已验收。 |
| 资料与数据源 | DataSources 管理标签 | Home/数据源二级页 | MacDataCenter | DataWorkspace/DataSources | 真CRUD/访问策略入口保留；添加→编辑→检查失败→恢复→删除确认全链待证。 |
| 模型与提供方 | ModelCenter/ApiModels/设置 | Models/IOSModelConnections | MacModelCenter/连接 | 设置/模型中心 | 真实连接管理存在，普通真实模型已证部分路线；本地模型安装、断连恢复与所有模型路径不因此完成。 |
| 计划与运行历史 | `Automations.svelte` create/edit/run/toggle/history | ScheduledActions/历史 | MacAutomationCenter/RunHistory | ScheduledActionsScreen | 真实入口及命令存在；本批尚无四端完整配置→运行一次→真实历史→删除确认点击证明。 |
| Skills/MCP/系统访问 | Capabilities/设置 | 上下文Skills/设置/系统授权 | Extensions/SystemAccess/Diagnostics | context Skills/Settings | 保留真实管理。操作系统权限由宿主负责；原型模拟开关不是新权限。完整管理覆盖待证。 |
| 活动/通知/结果 | Results/Dashboard/Today | Records/通知/Today | ActivityCenter＋全局活动列表 | NotificationCenter/Today | 真事实入口存在；所有需关注类型的筛选/来源跳转/补输入/失败操作待逐类验收。 |
| 设置 | 分类页＋内容标签 | Settings二级 | 原生Settings/Resources | Settings二级 | 入口保留；未逐个设置项真实点击，不能写“全部设置验收”。 |
| 工具授权与恢复 | current exact request/controller | typed Host/local wait | exact permission dock | exact permission component | Core事务/身份已有测试，部分实际工具任务成功；GUI全分支见 A12。 |
| 多专家 | complete manifest/新目标重发 | native完整集合/显式恢复 | complete manifest/独立owners | 完整集合/同owner真实长输出 | Mac真单专家＋重发4Run通过；真正多专家同时成功/一失败/停止/冷恢复仍需逐端证据；A01未闭。 |
| Mac Git review | Web真只读workspace diff | 无等价Git管理 | 实际Git文件/diff/提交/推送入口 | 无等价Git管理 | Mac既有提交/推送仍可达，但本批没有真实Git提交推送验收；原型从不执行Git，不能拿原型模拟通过替代。A02仍在。 |
| 普通对话与手机列表 | 独立列表＋搜索 | 独立主屏/返回 | 左列表/独立草稿 | Conversations独立路由 | 真实导航/输入已有部分验证。会话恢复/全文与草稿本批Mac通过；移动实际覆盖见下表。 |
| 移动个人助理架构（矩阵两行合并） | responsive Today | IOSPersonalWorkspace | 不把Today放大成桌面 | TodayScreen | 真实事实分区接线；自动整理、备注、稍后未实现，见A09。 |
| 桌面唯一顶栏 | 48px＋同栏tabs/tools | 不适用 | 48px＋同栏tabs/tools | 不套用桌面栏 | Mac真实窗口和Web部分浏览器已证；超长tab/1000px/大字体四端不等于全覆盖。 |
| 内容与侧栏隔离 | per-owner retained tabs | 手机独立导航 | per-conversation tabs/draft/scroll | 手机独立导航 | Mac会话实际切换＋草稿已证，Web生产管理页owner行为局部已证；PDF位置见A08。 |
| 审阅/产物 | 真diff/引用 | resources有限 | 真diff/附件预览 | resources有限 | 只有子集；完整深度见A02/A03。 |
| 进度/目标 | goal/plan/task事实及控制 | progress含goal | 两工作工具接真Goal/plan | progress含goal | 不虚构任务；依赖图、审阅/版本验收关联等未闭，见A04。 |
| 专家身份与颜色 | frame＋8色 | 外部身份＋Shared8色 | 外部身份＋Shared8色 | 外部身份＋8色 | 生产接线；独立颜色保存/取消有部分实际控件证据。所有八色×两主题×字体/320px当前产品实测未齐。 |
| 消息外部操作 | ChatMessageActions | ChatBubble actions | MacMessageView actions | MessageCopyAction/Edit | 复制有失败手动读取路径；Mac真实AX编辑已证，移动复制拒绝/系统不可用尚待证；失败重试见A01。 |
| 用户编辑重发 | Core prepare/fresh准入 | 普通＋expert独立editor | 普通＋expert实际全链 | 普通＋expert caller | 普通/专家源码闭合不是各端真实验收；Mac4Run通过，Android尚未真点编辑，iOS native与Webexpert仍按root当前结果补记。 |
| 持久化与边界 | per-owner state与真实Store | 本地偏好＋Core | 本地偏好＋Core | 本地偏好＋Core | 生产持久化比原型页内模拟更强，不能为了“像原型”丢Store；无声称专家色跨账号同步。 |
| 文档同步 | 当前基线存在 | 同一规范 | 同一规范 | 同一规范 | 实施总账含阶段性旧状态，后续必须用最新证据覆盖，不能挑旧“待测”当现失败或挑旧“通过”覆盖新修改。 |

## 3. 已有实际证据与明确边界

| 平台 | 已有生产/实际证据 | 没有因此证明的部分 |
| --- | --- | --- |
| macOS | `/tmp/nola-mac-ui-validation-20261028/FINAL.md`；35/35整组原生/contract＋最新22/22；`/tmp/nola-mac-real-memory.5OkisL`原生输入/AX专家发送→改当前模板/归档目录→原快照新Room重发→普通新会话/重发，4个实际answered sealed Runs；`jnCS8a`真实长流式332样本/26个AX树长度、会话切换草稿隔离/最新锚点，1个answered seal。 | 未覆盖A01-A12所有深业务。专家目前真实单专家，不是多专家并发混合失败。没有把旧独立Room截图算验收。 |
| iOS | `/tmp/nola-ui-ios-20261028.Oa9wmz/IOS-VALIDATION-STATUS.md`；初始contracts41，workspace真实控件12/12，scroll-disclosure-final9/9；`/tmp/nola-real-mobile-host.WhIP5f/ios-real.xcresult`实际primary qwen两次200、read_file、5段递增输出、唯一最终气泡、切执行后端、进程重启恢复、Back→Today→模型中心；同DB Core exact answered seal/全文/tool核验，wrong-marker负测拒绝。 | 此真实路线是mobile encrypted Web Host；native cloud专家/多agent长任务/冷恢复/跨会话草稿正在由iOS代理单独验证，不以普通Host结果代替。 |
| Android | `/tmp/nola-validation-20261028/android-real-three-verified-source.log`：真实expert107 running＋nonempty样本，UI全文1458与Core exact child一致，Activity recreate；真实stop→新发→恢复通过；`android-production-final.log`旧15/15为当前production路由/当前ExpertReply/JNI。 | 最新ordinary fixture曾假定Use但UI已Selected导致未发请求，代理正在补；Android编辑重发没有真实模型点击结果，不能用Core txn代替。旧deadRoomScreen的8张图已撤回。 |
| Web | root已实际Chrome专家管理创建/修改/归档/恢复、颜色保存/取消、/与@、生产普通真实回合/编辑重发；生产组件SSR/协议、类型和构建记录在实施总账。 | root正在补专家完整真实链；组件harness的popover/fixture A2UI流式不等于完整产品真实模型；未逐项验证全部管理页/深工作工具。 |
| CLI | 父任务已真实PTY执行help/即时命令/退出/恢复；delegated故障注入4/4；共享Core最终套件由Android代理集中报告。 | 普通CLI真实操作不证明GUI；原型不要求CLI有点击/三栏/手机Today。Goal旧fixture剩余问题依父验证账，不在本次审计编译。 |

## 4. 后续实施边界

1. 优先修现有能力丢入口/平台差异：A06/A07、手机已有真实工具组织、PDF位置、字号/标签与完整管理操作验证。
2. A01/A02/A03/A04交由统一Core方案；不在四端另造失败重试/发现审阅/产物版本状态机。
3. A09需产品业务与共享事实闭合后实现，继续只展示真实Today快照，不能填样例日报。
4. 按每条差距追加“生产调用→真实操作→精确持久结果”证据；保留失败与不适用范围。当前goal仍有未完成项。

辅助设计依据：apple-design技能本次读取的 `layout.md › Visual hierarchy` 强调信息分组和渐进披露；`toolbars.md › Best practices` 要求工具可发现且不拥挤；`sidebars.md › Best practices` 区分导航与上下文。Android/Web采用这些通用原则，不套用Apple专属平台控件。该辅助准则不替代仓库原型要求。

### 本次审计后的窄实施进展（尚未验证）

root 已将 A06/A08/A11 的可复用部分交 macOS 补齐：专家创建/编辑模型继承与选择、现 PromptTemplateService 目录选择及现有模板管理编辑入口；PDFKit 页/缩放/阅读锚点按会话和对象保存；消息小字号提高至12pt。实现及回归源码已落盘，**尚未编译/未运行，不覆盖上述通过证据**。QuickLook 不公开通用文档内部阅读位置，本次 PDFKit 方案不宣称覆盖任意 QuickLook 文档。

A01 的下一批共享设计见 [失败专家新尝试](EXPERT_FAILED_ATTEMPT_DESIGN_20261029.md)，仍未实现。root 最新补充 Web 完整生产 API＋实际模型 ordinary/普通编辑/两专家完整集合/两专家编辑新child均通过，`protocol-current-report.json`；此为生产 API 模型证据，不因 Chrome 当前不可见而冒称新浏览器点击。

### 关键生产定位

- 专家管理差异：[Mac manager](../../NolaMac/Features/Chat/MacAgentRoomsView.swift:305)、[Android manager](../../android/app/src/main/java/com/nola/ui/screen/chat/ExpertManagementPanel.kt:29)、[iOS manager](../../Nola/UI/Chat/IOSAgentRoomsView.swift:4)、[Web manager](../../web/frontend/src/components/ExpertManager.svelte:1)。
- 工作工具子集：[Mac artifacts/review](../../NolaMac/Features/Inspector/MacInspectorWorkTools.swift:51)、[Web artifacts](../../web/frontend/src/components/ChatArtifacts.svelte:1)、[Web review](../../web/frontend/src/pages/WorkspaceReview.svelte:1)、[iOS inspector](../../Nola/UI/Chat/IOSConversationInspector.swift:4)、[Android inspector](../../android/app/src/main/java/com/nola/ui/screen/chat/ConversationInspector.kt:28)。
- Today事实：[iOS](../../Nola/UI/IOSPersonalWorkspaceView.swift:195)、[Android](../../android/app/src/main/java/com/nola/ui/screen/home/TodayScreen.kt:39)、[Web](../../web/frontend/src/pages/Today.svelte:1)。
- 普通失败/投递恢复的差别：[Mac pending retry](../../NolaMac/Features/Chat/MacAgentRoomWorkspace.swift:609)、[Web selected/executing恢复](../../web/frontend/src/pages/Chat.svelte:1141)。行号仅为本次定位，后续改动以名称为准。

原型中研究/写作/推理三位专家是演示样本，生产不应为满足颜色示例向用户目录插入三位假专家；生产默认色与可配置八色须分别核验，不能把样例数量当业务需求。

### macOS 窄补验证结果（覆盖前述“尚未验证”状态）

`/tmp/nola-mac-ui-validation-20261028/contracts-19.xcresult` / `contracts-19.log`：当前批次 **40/40，0失败、0跳过**。包含共享 selector 胶水字段保留、真实 Store 的专家模型/模板配置 CAS 和既有 context 不变、原生 PDF 第4页/缩放/destination 恢复、6页切1页与连续 B/C owner request 的旧写入隔离、原生窗口中未存表单 Create/Done 的取消/丢弃及实际目录读回。浅深色当前管理/回复视觉图在 `/tmp/nola-room-connection-visual` 已人工查看，字段与中文文案没有裁切。

首轮 `contracts-18` 40项有6个失败，完整保留：一个未注册模型 fixture 被 Core 正确拒绝、三个 AX informal 返回值假设、PDF 新 owner 无位置却沿用旧文档坐标，以及该问题影响后续视觉。已修 fixture 为真实 Store profiles，AX 仍核实际关闭/字段/目录状态，PDF 新 owner 无快照明确从首页开始；未放松 owner/token/Core 守卫。

本批并未新增真实模型调用；此前普通/专家编辑与流式真实证据仍保持其明确范围。模型/模板弹出选项逐项真实选择与模板库编辑的完整 GUI/实际模型组合仍不能由该40项自动推断。A01/A02/A03/A04/A05/A09等缺口未被这些结果关闭。

## 5. 后续主审新增的边界（不并入已通过计数）

- 专家姓名：root导出contracts-19原生附件后确认，Mac48px栏的长姓名仍被ellipsis截断，iOS38px栏有竖排风险。Shared新增纯视觉AppleExpertAvatar/AppleExpertName，两端迁移为装饰头像＋气泡上方全文宽列、唯一AX姓名；当前正在受影响单方法/原生验证。旧40条通过不能替代新增布局验证。
- 模板执行授权：新Core可读目录只约束选择和展示。现`expert_message::custom_template`在完整准入时仍直接按ID取正文，尚未携带可信宿主当前resource actor并核内容权限；缺目录/ID也可能按内建路径留给Host。不能把目录过滤或客户端保存前复核写成执行授权已完成。后续需Core在真实定义捕获边界核可信actor/资源身份/版本与可读内容，明确内建与自定义来源；不能由Renderer伪造owner、默认为全部本地资源所有者、沿用旧授权或用ID关键字判路由。当前版本记录必须明确拒绝不匹配，不自动迁移或认领数据。
- 稀疏模型参数：Android旧`readConversationParameters`要求context/output/temperature/topP四字段齐全，否则整份override返回null，正常expert factory可能回落profile参数而丢掉合法稀疏temperature。新selector仅改选择键并保留所有旧参数，不伪称已解决运行时解析；后续应复用Core当前generation/profile语义，补实际请求字段验证，不能用UI默认值覆盖用户稀疏设置。
- 冷恢复：真实iOS任务发现旧240s lease等待。新增共享精确OS process证明与同快照校验/写CAS已实现，Core SQLite12条和CLI3条回归通过；iOS/Android前台正在接线重建后真实kill/relaunch。活/Unknown保留当前expiry路径，外部未知副作用仍走原重放安全策略。

### Mac 专家姓名后续验收（11:34）

当前 `AppleExpertAvatar` / `AppleExpertName` 共享展示组件接入后，仅补跑当前生产专家消息组件的窄宽明暗两态单方法：`expert-name-24.xcresult` **1/1 passed、0 fail、0 skip**。NSWindow 实际截图与原生 AXStaticText 同时验证完整姓名只出现一次、姓名列宽度大于 100，不把头像栏首字当第二份姓名；已目视明暗图均无姓名省略/遮挡。证据固定于 `/tmp/nola-mac-ui-validation-20261028/expert-name-24-artifacts/`，日志 `/tmp/nola-mac-ui-validation-20261028/expert-name-24.log`。这只覆盖本次身份展示修改，不扩大为新一轮全端或真实模型验收；此前失败构建/AX 激活问题记录保留。

### 真实 Responses 思考流后续定位（12:40，未冒称验收通过）

Android 原配置普通长任务在输出上限前收到约32KB公开 `reasoning_text`，但旧适配器只接受 `response.reasoning_summary_text.delta`，导致实际思考没有进入展示链路。此次是明确的外部协议适配缺失，不能用“模型未思考”解释。Android正在修复并重验；root同查到Apple共用OpenAIProvider存在同样遗漏，已补 `response.reasoning_text.delta`、终态公开content补齐及按item/index去重。加密continuation仍原样保存且绝不作为思考显示，矛盾snapshot/模糊多item身份拒绝。Apple新增真实SSE帧消费回归及公开/加密边界回归，当前只完成Swift语法解析，尚未编译执行这些测试。

iOS原生两专家任务已取得两份真实完成seal，持久化正文分别4212/3928字符；只读滚动UI验收仍有一条正文未确认可达，继续定位，不以Store完成替代界面完成。CurrentWork真实六路导航及源草稿走过，模板选择/取消的AX断言和实际取消入口仍在收口。

本次用户再次打开Chrome并保持解锁后，桌面工具仍只列出运行进程、浏览器列表为空，选择com.google.Chrome返回cgWindowNotFound。该环境结果不计为当前完整产品浏览器通过，也不反复要求用户解锁。

### Mac 生产父消息归属投影（后续窄修）

删除只被旧测试调用的 `MacRoomTranscriptRow.project` / `TimelineEntry`，当前 `MacUnifiedExpertTranscript.rows` 已调用 `MacExpertTranscriptProjector`：保留 Core 父数组顺序，按 receipt 精确 room/conversation/parent/sender/context/Run 归属把 child reply 放原 parent 后，同行专家用当前关联的稳定顺序，不按开始时间/到达时间排序。纯展示 `canonicalMessageID` 由现 repository Core 投影填写，显式排除 `Codable` 存储字段。

`/tmp/nola-mac-ui-validation-20261028/expert-parent-order-26.xcresult`：**27/27 passed、0 fail、0 skip**。包含真实 Store 12条用户消息逐页 canonical ID producer 断言、生产投影晚回复/跨context同UUID/增量身份/foreign与partID冒充拒绝，以及显式归档omit。没有重复姓名25、模型或真实多专家 GUI，本批结果不覆盖它们。

### 本轮后续真实结果（更新上述待验证项）

- macOS `expert-parent-order-26`：受影响Workspace两suite及真实Store窗口→repository producer共27/27、0失败、0跳过。父消息仍用Core顺序，专家子消息按精确父锚点插入；新canonicalMessageID实际由Core投影赋值，明确不进入归档wire。未重跑真实模型。
- iOS `59358`对应只读真实Store界面案通过：第5页实际物理drag遇到第二位专家，两份已完成正文4212/3928字符逐值（仅折叠空白）匹配，两canonical child与全文姓名均验证；实际Latest、两会话独立草稿及再次kill/relaunch恢复通过。此前第二条未找到是测试没有遍历到，不构成已证实的生产丢消息。`native-terminal-scroll-and-drafts.json`保留当前passed结果；管理确认仍有Cancel AX父/子重名selector问题，未计通过。
- Apple `OpenAIProviderTests`整suite通过，包含本轮4项公开reasoning SSE/terminal补齐/加密不显示/item-index alias与冲突回归。语法解析不再是唯一证据；这不自动扩为iOS实际模型思考流验收。
- Android原配置原任务实际模型重验：思考区展开且屏幕bounds可见的live非空样本571个，公开思考从3增长到1611字符，证明发生在terminal之前；实际截图已由root目视。原20行正文任务仍未在原限时内完成，保留失败，不换模型、不关闭工具、不降要求。JVM652项、0失败、2跳过，其中8项当前Responses协议帧回归。`android-real-current-artifacts/plaintext-reasoning-ui-timeline.json`及同名png为证据。
- Android普通/专家真实编辑重发新增2案源码：只复用有精确answered seal的真实source，经UI点击编辑/取消/提交，核新owner/新child、原定义冻结、原会话不变、实际seal和界面全文及Activity重建。尚未编译运行，不计通过。

仍观察到待收口的iOS旧等待卡（已admit输出时仍显示Resume）及请求恢复误标Model fallback。将修精确事实刷新与共享展示分类，不以删除诊断或伪造成功绕过。

### 恢复提示与编辑入口本轮收口

- Apple ModelAttempt 已共用纯展示决策：只有真实 successor attempt 关系才表示重新请求；未知路线只称“请求已重新开始”，已知路线变化中性展示，单独出现后续模型名不推断重试。原 provider namespace／device_local 不再拼成主模型名。无 successor 的真实 failed 才红色。Mac共享3项、Journal2项及投影17项均取得通过证据；投影首轮4个当前schema fixture错误修复后 `projection-fixtures-29` 精确4/4通过。`model-attempt-27`首轮失败和28零用例结果保留，不描述成单次全绿。
- iOS stale Resume 按完整manifest和Core exact Run事实刷新；本机inflight期间不误示待恢复，实际receipt/admission边界coalesced重读，旧owner迟到不回写，不由单个peer掩盖其它缺失peer。当前11项focused case均已实际通过：管理模板保存重开、Cancel/Discard真实操作，ModelAttempt与peer/owner及取消cleanup回归。27368旧fixture失败保留；48453精确3项通过后才关闭该项。
- Android ModelAttempt同语义，当前JVM658项0失败/2跳过，生产16项再次通过。真实stop→下一轮AFTER_STOP=703→Activity恢复单案通过（20.105秒）；真实隐藏键盘后整窗 `imeVisible=false`，composerBottomGap=0。先前Compose root截图未包含IME，不构成已证实的底部布局缺陷，也未为此误改产品insets。
- Android编辑重发已经真实进入提交，发现Android自己产的模型能力wire漏 `native_pdf_input=false`，与Core冻结完整当前profile严格比较失败。该次没有伪造准入或改用当前模型，保留真实拒绝；正在补显式当前字段并保持true／未知能力拒绝。新增editor.error语义节点让测试记录明确生产拒绝后立即失败，不再在已知拒绝下空等模型deadline。

模板捕获授权新批：Core 同事务捕获、只读预览与模型 revision pin、实际 Begin 定义集合核对、私有来源事实、C ABI / service / CLI-Web 操作源码已接入；Apple / Android 接线与审计仍在进行。本批尚未编译、未运行，不记为生产已完成。不在既有 Store 中自动补 owner、补授权来源或转换旧记录。

### Android 真实编辑重发最终补证

- 当前 JVM 660 项：0 失败、0 错误、2 跳过；当前生产 Compose / JNI 16 项全部通过。以上属于前一 UI 批次，不覆盖正在修改的模板授权批次。
- 普通真实编辑重发 r7 通过；专家真实编辑重发 r8 通过（22.804 秒）。均由实际界面新建 source、调用配置的真实模型并取得 answered seal，再验证取消无写入、编辑产生新 owner / child、定义冻结、实际回答全文、Activity 重建以及原 source 完整投影与 manifest 不变。
- Android 漏写 `native_pdf_input=false` 的当前能力字段已补齐，继续严格比较完整冻结模型能力；true／未知且平台不支持的能力仍明确拒绝。提交中进度与错误分为不同语义节点，不再把正常 0/1 确认进度误判为业务拒绝。
- 真实 reasoning 流曾取得 571 次可见非空采样（3→1611 字符），但该次长任务 180 秒内未得到最终回答；仅证明实时思考展示，不将整个任务记作成功。Activity 重建也不等于操作系统强杀恢复验证。
- Chrome 再次尝试连接：能发现进程，但原生窗口绑定仍返回 `cgWindowNotFound`，浏览器连接列表为空。完整 Web 产品的本轮真实 Chrome 点击仍未取得，接口与组件测试不能替代此项。

### 模板私有 Host 批次：首轮实际验证

- 三端源码已冻结并经主代理审查后才启动本批编测。新增共享 Apple pending journal / owner 集合、原请求对账、同文新草稿 ticket；手机端不以 free/open 句柄冒充实际共享 owner 重开。
- `core-template-capture-current-r4.log`：真实 Store 专家准入 / 旧定义保留 / grant 与模型漂移 / 实际 Begin / commit 与 savepoint 故障等 26/26 通过。先前回答 Item ID 跨专家冲突的 fixture 失败与预览只读检查失败保留；后者定位为真实建库归档后台任务仍在提交，现等待其实际 publication frontier 归零后再核原 `data_version` 不变，未删除只读断言或停用后台任务。
- `core-host-template-unit-current-r2.log`：Application unit 399 通过、1 忽略；bootstrap 14/14；mobile bindings 26/26。包含真实 deferred-FK commit 故障后两个**仍存活** native handles 用同 request ID 恢复且只提交一次，以及重开失败保持 absent / 内存库拒绝换空库。
- 同一运行中 Protocol 31 通过、7 失败：冻结 JSON fixture 仍写 52，当前严格协议已为 53。正在修 fixture 为唯一当前格式并重跑；没有放宽生产版本检查。该运行因失败未执行后续 service suite，不能记作整组全绿。
- 后续 `core-protocol-service-current-r4.log`：Protocol 38/38、service 18/18 全部通过。fixture 已更新至唯一当前 53 并归入 `fixtures/current`；service 的 Task admission fixture 改用当前 Domain version 常量。原失败记录保留。真实 owner 不可用时原 mutation 返回未确认、commit 不确定后原 owner 重开、savepoint 失败退休与 idle wait 行为均实际执行通过。
- `web-template-host-check.log`：Svelte 0 错误 / 0 警告；`web-template-host-tests.log`：563/563、0 跳过。未覆盖新 Host 实际模型和 Chrome 点击。
- Apple 原生编译 / 新 XCTest、Android Kotlin / JNI 和实际操作、CLI / Web 新二进制验证仍待本批后续执行。有效 sparse 数值参数、A01 等既有差距未因此关闭。
- iOS `template-host-current.xcresult` 与同名 summary：当前源码 Test 配置编译成功，6 个所选 suite 合计 40/40、0 跳过。包含真实隔离 Core Store 的明确拒绝与 private journal 清理；不等同于新模板下真实模型点击已通过。
- 新 Host 构建成功后在新隔离 Store 实际调用配置模型：普通对话取得 verified terminal；普通编辑因工厂身份误进入 portable runtime 被严格拒绝，双专家因实际 `web-response-style` 未登记而 Begin 被严格拒绝。报告 `web-template-host-current/protocol-report.json` 保留两项真实失败；未放宽 Core 完整定义比较。对应 CLI/Web 源码已修，等待重建与实际复测。
- 后续生产构建 `host-template-current-build-r4.log` 成功；`web-template-host-r2/protocol-report.json` 四阶段实际模型复测全过：普通、普通编辑新 owner / 原文不变、完整双专家、目录改名归档后按原快照重发双专家。工厂身份仅内存使用后清除且不序列化为 portable runtime；Web 实际风格 / custom 生成器供捕获与发送复用，实际读取并提交 Core 完整定义集合。原失败未覆盖删除，API 证明仍不冒充 Chrome 点击。
- Android `android-template-host-jvm-current-r2.log`：当前 Kotlin 源码编译通过，JUnit XML 共 668 项、0 失败 / 0 错误 / 2 跳过；新 JNI / 模拟器操作验证另行进行。首次缺 Material3 import 的编译失败日志保留。
- CLI 新增三项实际执行各 1/1：工厂身份不进入 portable runtime / 不能由序列化输入恢复，真实 Web 指导生成器，SSE 明确错误不被缺失 Run 的下游查询掩盖。`cli-expert-factory-contract-current.log` 初次 exact selector 为 0 用例，未计通过；后续三个 `*-verified.log` 使用实际列出的完整测试名执行，均确认 1 项通过。
- Mac `template-capture-30.xcresult`：当前 Test 配置 / arm64 生产 App 与测试 target 编译完成，27/27、0 跳过（RoomWorkspace 18、EditedResend 5、ExpertTemplateHost 3、CanonicalPrompt 1）。
- Mac 新隔离 Store 的真实 UI / 模型单案 `/tmp/nola-mac-real-memory.pc8JcM/result.xcresult`：1/1、0 跳过，4 个 Run 均 answered / sealed。实际操作 owned 模板、`@` 专家、修改当前模板与归档后原快照重发、普通编辑；`capture-semantics-checks.json` 9/9 确认新 owner / context / Run、原管理定义 SHA 不变、custom ReadContent 来源、retained 不携旧决定。root 已目视 `expert-edited-ui.png`：697 全文、专家身份一次、原会话列表可见；单帧不代表长任务、所有尺寸或其它管理路径均过。
- iOS 真实 UI 准备发现 `PromptTemplatesView` 原来只有自身 Preview caller：已新增专家管理内的 44pt 二级模板管理入口、保留原编辑草稿、返回时真实 reload，以及编辑器最少 AXID。此增量待编测 / 实际创建，不能用前一批 iOS 40 项代替它。测试初始化仅准备真实模型的 style / custom 参数；模板须由实际 UI 创建，不能直接造 owner / grant 记录。

### 当前真实操作续验（保留失败与未完成范围）

- iOS 连续三轮实际操作暴露模板库、选择器分别只观察外层容器而不观察模板服务的问题：实际保存成功，但当前列表或选择器不刷新。目录子视图与选择器现分别观察同一个 `PromptTemplateService`；前两项修复后的第四轮正使用新隔离 Store 与原模型执行。前三轮在模型请求之前失败，不能计作真实模型或完整专家任务通过，也没有通过延长超时或直接写入模板来绕过。
- Android 新 JNI / 新 APK 的生产界面操作 `android-template-owner-gate-production-16-r3.log`：16/16，通过耗时 139.319 秒，覆盖 IME、管理、模板、归档与切会话草稿。切换目标 owner 时先显示加载态，草稿只在目标配置与草稿均完成加载后可写；同一会话生成期间仍可编辑下一条草稿。此前冷 IME 与旧 owner 操作失败保留。该轮不能证明所有竞态时间窗都曾实际触发。
- Android 后续真实编辑任务发现：编辑重发内部仍按旧条件等待目标 owner，目标 ID 在新加载态已出现，因此先于配置加载触发原 `blocksInput` 检查。保留真实失败，拟等待实际配置就绪后继续；不放宽 Core、活跃 Run 或权限准入。当前不计普通编辑重发通过。
- 用户再次打开并解锁 Chrome 后，工具仍报告 `cgWindowNotFound`，直接创建 Chrome 标签页也返回 `Browser is not available: chrome`。没有取得本轮完整产品真实点击证据；已通过的 API / 模型任务与组件测试保持各自范围。
- Web 专家消息确认改用现草稿 store 的 owner / edit revision，不再通过文本相等清除草稿。沿原写入队列完成带 revision 的持久化后才清除原专家 intent；存储失败、确认期间新编辑、同文 ABA 编辑均保留对应草稿。主审后受影响两个 suite 实际 20/20、0 跳过，Svelte check 0 错误 / 0 警告（`/tmp/nola-web-expert-draft-20261029-tests.log`、同前缀 `check.log`）。这些回归使用可控存储接口，不冒充 Chrome 点击或 IndexedDB 浏览器集成验证；冷启动缺原本地编辑 ticket 时保守保留草稿，范围见 `WEB_EXPERT_DRAFT_ACKNOWLEDGEMENT_20261029.md`。
- iOS 第四轮 `/tmp/nola-room-real-ui-44cc761d-a8af-4c6e-a128-f186f141de38` 已结束，exit 65，单案 491.057 秒。实际 UI 创建模板与两位专家、两份真实 Begin、完成前文本增长、强退重启均已越过旧失败点；一位专家已 sealed 且界面完整正文已采集，另一位在原恢复窗口内仍 running。最后实际 HTTP 为 200，公开 reasoning 持续输出但没有正文 delta，且无 pending permission，不能称审批阻塞或未发请求。整项长任务 / 双专家完整终态验收仍未通过；保留原失败，不延长阈值、不换模型、不自动重跑。

### 由真实截图与请求继续定位的增量（尚待本批编测）

- iOS 公开思考流的第一丢失边界已定位在 `OpenAIProvider.generateEventStream`：实际是 Chat Completions，Core 计划 `reasoning=false, stream_reasoning=true`，但 Provider 用请求的 `enable_thinking=false` 丢弃已收到的公开思考；`TextAgentModelClient` 接收与完成处还会再次按控制能力清空。请求控制与接收公开输出必须分开，正在只修接收边界；不修改模型能力或请求策略。不能把前一轮文本增长说成思考流已通过。
- root 已目视强退前、恢复窗口末两张真实截图。专家长姓名完整且只显示一次；恢复卡的原始 `retry_or_fallback` 仍直接占主流程，现已移入默认折叠的诊断详情（7 语言），真实 failed 且无 successor 的主错误仍保留。业务状态、原诊断与 Run 均未更改，已完成第二代理静审，待本批编测。
- Android 更多菜单新增直接“上下文与来源”，复用既有页面与原数据读取，未新增工具栏图标或第二业务状态。只在目标配置已就绪时开放，返回只关页面并保留原草稿。菜单与返回/owner 语义经第二代理静审；A06 增加实际点击、关闭与同草稿断言，和编辑重发的加载等待修复一起增量验证。
- Apple 公开思考修复的 focused 首轮 `public-reasoning-focused.xcresult`：38 项通过、1 项 fixture 失败。新 Chat Completions 双公开字段的输出断言已过，失败在 observer 假定请求只能是 `httpBody`；Foundation 实际合法使用 `httpBodyStream`。测试仅补读取该流后保留 JSON / 请求控制原断言，精确重跑 `public-reasoning-request-stream.xcresult` 1/1 通过，累计 39 项实际通过、0 跳过。请求构造未改变；首轮失败保留。这涵盖实际 URLSession/SSE 适配、模型事件公开/加密边界及 ModelAttempt 分类，不等于真实模型思考展示或折叠控件点击已通过。
- Android 本次 Kotlin 增量 29 秒完成（跳过原 JNI/NDK）；A06 的直接上下文项已真实点击，但新断言没有等待转场完成而立即失败，保留该 33.493 秒结果，不能推断页面实际无法打开。正在补正常界面可见等待，原模型与 IME 限时未改变。真实普通编辑重发已通过该轮用例，专家用例仍在执行，暂不合称两项通过。
- Android A06 补可见等待后仍失败，进一步源码与真实操作共同确认**确有生产门禁错误**：整个 ContextScreen 仍嵌在 `if (inspector != null)` 内，而直接入口先关闭 inspector，导致页面根本不组合。root 与第二代理原静审均漏掉这层条件，不能将前一次失败归结为纯转场问题。现改为 `inspector != null || showContextSheetOpen`，保留原导航/返回与所有断言，只重跑 A06；两次失败均保留。
- Android `android-template-permission-real-edits-r5.log` 对应真实普通 / managed 专家编辑均通过：独立设备 TestRunner 明确 2 tests / 0 failed，`android-template-real-r5-artifacts/{ordinary,expert}-report.json` 均 passed，专家实际 Begin 完整定义核对通过，当前目录改变后新 child 仍用原定义。前台 am 流未返回完整摘要，保留该现象并使用独立 logcat 与 report 交叉核实，没有重启以凑摘要。该轮模型没有请求审批，严格一次批准 driver 虽已编译，**实际审批分支仍未覆盖**，不能替代 r4 等待卡的点击证据。
- 后续 A12 独立真实任务已补齐上述一次批准分支：同配置模型自主选择 `render_ui`，测试读取当前 exact Run / request / call 和纯文本组件参数，再实际滚动点击 Allow once；Core 确认 `allow_once`、工具 completed、answered seal、同 owner 卡片的 marker 与 703。`android-real-inline-a12-r2-artifacts/report.json` 与原请求摘要对应，root 已目视 `permission.png`，按钮完整位于输入区上方。首轮因测试未接受当前 catalog 合法 Text.variant 而拒绝，保留失败；仅按仓内 v0.9.1 catalog 的七种样式枚举补观察器，没有放宽生产权限。
- A06 r6/r7 的实际导航与草稿行为通过，但整窗截图仍是前一帧，未据此宣称像素验收。随后只读重开同一已完成真实任务、无新模型请求的窗口诊断 r2 实际 1/1 通过（26.594 秒）。`android-actual-window-r2-artifacts/actual-windows-6d2752b7-e32d-490e-b787-a28fc1fb1c38.json` 记录 node PixelCopy / 整窗两层、当前 user/display、窗口可见性及设备 SHA；拉取后 SHA 均一致。root 已目视 Context 真实页与原卡片整窗，确认页面与 marker/703 卡片确实绘出，不再仅依赖语义节点。此前诊断列表未加载的失败保留。卡片顶格无消息边距、同消息多段思考的组织与位置仍需按实际 canonical order 继续审查，不能因卡片存在就关闭所有排版问题。

### 共享有效参数查询批：实际验证与尚未迁移边界

新增查询与专家 preview 复用同解析器、统一只读事实 port，Shared / Android 必填 `effective_parameters` 已同步，缺字段没有默认值或兼容别名。query 首轮 8/11、第二轮 10/11；失败均保留。修正夹具使用真实 Host 默认继承路径来创建 sparse 配置：显式选择模型当前会在写事务初始化完整快照，不能拿它假装创建出 sparse。reasoning 夹具按当前 concise/balanced/detailed 与完整 wire low/medium/high 声明修正；未放宽解析器。

`effective-parameters-query-r2.log` 的 10 项与 `effective-parameters-query-r3.log` 的最后单项合计 11 个不同查询用例通过；专家事务/preview 27/27、纯解析器 4/4，mobile bindings/bootstrap/service 三包编译检查通过。非 object profile source 明确拒绝且不改写，修复旧 JSON Value 索引可能 panic 的边界；本机可核的错误 agent-host profile pin 也明确拒绝。**这些不代表原生参数 caller 已迁移**：本机模型/源 Host 权威 baseline、各端 effective DTO 到真实请求的接线仍未完成。新的 Apple FFI/Test 与 Android 必填 DTO JVM 验证另行登记，不复用旧 JNI 的 UI 结果冒充新查询通过。

- 新 Core / FFI 下 `effective-core-reasoning-focused.xcresult`：iOS 四个受影响 suite 46/46、0 跳过。实际编译日志与 UI 测试产物 symbol 均确认包含新真实思考测试，后续没有伪造缓存时间。Mac `effective-core-32.xcresult`：同本机 arm64 / Test / 原隔离 DD，27/27、0 跳过；未触 Products/Debug，未重跑此前四个实际模型 Run。Android 必填 DTO JVM `ExpertCapturePreviewContractTest` 2/2、无失败或跳过；明确未因此重建新 JNI。
- 新 iOS 真实公开思考 / Stop 案 `nola-room-real-ui-13aac7a6-62dc-4e14-9099-1fb197ffced5` 有一个真实 native Run，151.733 秒后测试失败。root 已目视公开思考正文；Core 存在 1914 个公开思考版本，供应商实际返回 200 与公开 reasoning delta。这证明公开流不再于前述适配器边界全部丢失，但**不证明可见增长 / Stop 整案通过**：测试误取折叠标题，且实际 Store message/Run 已 running，界面 AX 仍 pending。Stop 尚未到达，未伪造取消 seal。正在修精确 child metadata publication；不得仅放宽测试状态条件来掩盖生产 stale metadata。
- 新顺序审计以实际 Android 已完成任务的 journal 为准：`Reasoning → Tool → UiSurface → Reasoning → Text → empty Text`。Core 顺序正确；Android、Web、Apple 的部分现展示按类型重组，或将所有 Surface 追加在回答后，属于客户端实现问题。Mac 专家路径另缺 Surface context 注入。Android 已落原序连续段、正常/专家共享 Surface slot 与任务详情披露修复源码，**尚未编测**；其他平台还需接线。不能把两个不同真实 reasoning step 按摘要相同去重，也不能为了简化改变 Core journal。


### 消息原序展示与真实 Stop：本轮新增证据

- iOS `native-metadata-state.xcresult`：`IOSAgentRoomWorkspaceTests` / `IOSPublicPartSemanticParityTests` 44/44、无跳过。native snapshot 只在精确 owner 的 assistant 元数据仍缺失或 pending 时重读 Core；已 running 不逐 token 查询，不从文本推断状态。
- 真实 iOS 案 `nola-room-real-ui-a810c0f7-a1dd-4f39-816e-9097562e1fcb`：屏幕中公开思考实际由 184 增至 557 字符，两份可见正文和 bbox 均为 running。真实 Stop 已点击，但原 45 秒内未确认取消，**整案仍失败**。精确 Run `5345cb56-a305-418f-afa3-579c84c7f405` 没有 interrupt receipt / cancellation event / terminal seal，上游仍自然完成；录屏点击后 4 秒已出现 Unknown error。定位到先推进 UI generation 撤掉 native observer、再异步取消的竞态；现源码调整为先提交精确 durable interrupt 再退休观察，迟到旧 owner 不清新 UI。该修复编测与真实 Stop 重验尚未完成，不以 stream EOF 代替 terminal。
- Android 新 JNI 构建成功，原序展示 JVM 10/10。首次 Compose 15 项中 10 通过、5 失败，原日志保留；其中 unmerged 语义树 selector 与 clipped bounds 属测试观察问题，不据此修改产品顺序。工具详情阅读后父段保留展开的 Kotlin 修复经审查，第二次 targeted 6 项 4 通过、2 失败；剩余 prepend/output 可见性和任务详情 viewport 正在进一步实测，不能登记整组通过。
- Web 已按原始 Part 连续段渲染，Surface 回自己的锚点；仅真实 pending/running reasoning 自动展开，早先完成段保持折叠，显式用户选择优先。相邻文本仍沿用既有 Markdown 拼接，空白/待审核文本、卡片等不跨越合并执行段。专家路径新增精确 context/message 的只读 Surface 展示，不借父会话模型或提交控制器。**专家卡片交互仍未接通，不能把只读显示记作完整交互验收。**
- Web `web-ordered-parts-related-tests-r3.log`：受影响展示 / Surface owner / tool disclosure / expert projection 25/25，无跳过；`web-ordered-parts-check-r3.log`：0 错误 / 0 警告。首轮类型检查 3 个错误和 readonly SSR 测试未读取 lazy `.body` 导致的失败均保留，修复后重跑；这些属于组件 / 协议验证，不是 Chrome 点击。
- 用户再次打开并解锁 Chrome 后，工具最新返回 `Native apps: Sky Computer Use native pipe startup failed`、apps/browsers 均空。真实 Chrome 产品验证仍受环境阻挡；未重复要求用户解锁，也未把 API / SSR 算作真实操作。
- macOS 原序段与普通/专家共用 Surface 注入源码正在完成审查，尚未编测；历史单 part 的完整安全版本读取接口也未收口，不把 bounded preview 当全文实现。


后续实测补充：iOS 取消排序修复的 `IOSDeviceAgentToolPlanLifecycleTests` 13/13、0 跳过，`native-cancel-order.xcresult` 保留实际 Coordinator / SQLite interrupt-before-detach 与迟到 owner fence 证据。完整真实模型 r4 仍在执行，尚未计 Stop 通过。Android targeted2 诊断第三轮仍 0/2：原 Tool 节点仍存在，但 HideDetails / 原 output 均为 0，明确是展开状态丢失；任务详情同 root 未裁剪坐标的先后顺序正确，但 After card 实际 scroll 后不可见，不能把顺序断言通过当可达性通过。两项按生产问题继续修，保留 `android-ordered-parts-r3-diagnostics` 的 AX、bounds、截图与原失败。


取消修复完整真实重验 r4 已取得终态：`nola-room-real-ui-fbc1f3c1-ad65-47e8-8ac5-7c4eb3dda86d` 的真实 UI **1/1 通过、0 跳过，81.760 秒**。root 已目视首个 / 增长 / 取消三张截图，公开思考 86→167 字符，实际 Stop 后消失；Core 原 Run `7364dde5-b6cb-470c-949d-b3d895f2041a` 为 sealed/cancelled，interrupt receipt committed，model/cancel delivery 均 settled。`native-public-reasoning-audit.json` 的 9 项独立审计全部 true，原模型/单 native Run/无 mirror/精确 assistant owner 均核验。上述“r4仍执行”是时间序列记录，由本段更新；r1/r2/r3失败不删除，本次也不替代原双专家长任务。终态截图仍存在一行与卡片两处 Response stopped，已安排随 iOS 原序展示批修复；当前 iOS 部件顺序及 Surface 锚点还不能算收口。


### 消息层阅读选择与详情面板后续批

Web 将 process 折叠选择提升到同消息 owner，按各 Part ID 保存；连续段重组时明确 false 优先，显式工具详情阅读使该段保持展开。普通视图 key 包含 Host / conversation / message，专家视图也含 exact context / Run，不按首部件重建后丢选择。`web-message-owned-disclosure-tests.log` 26/26、无跳过，`web-message-owned-disclosure-check.log` 0 错误 / 0 警告，交互测试页构建成功。新的 hit-target 页使用真实 ChatMessageContent，并提供前插/终态控件供浏览器验证；**尚未真实点击这组新增操作**。Chrome 工具已可再次列出进程，但绑定仍 `cgWindowNotFound`。

Android message-scoped controlled choices 修复后 pure JVM5/5，通过真实消息组件前插与直接组件前插两案；第三个详情诊断被双 root 取证错误打断，原失败保留。root 目视整窗确认初始 partial sheet 下存在屏外内容，但 `performScrollTo` 语义动作不能代表所有真实手势都不可达，不将其扩大结论。当前按两个既有 inspector 的设计统一指定 `skipPartiallyExpanded=true`，后续必须验证实际 swipe、逐项可达、Back 及整窗；尚未登记该修复通过。iOS 原序源码首轮审计未过：除 Text/UI 外的多个类型仍尾部 append，已要求以全部 canonical Part 原锚点复用现呈现，未启动这批编测。


Android 详情全展开修复 r5：`android-task-detail-expanded-compose-r5.log` **1/1 passed，11.671 秒**，逐 Part 未裁剪原序 / Before、卡片、After 实际 scroll 后可见 / 真实 swipeUp / Back 均执行。root 已目视 `android-task-detail-r5-artifacts/after-swipe-window.png`，After card 清楚可见；bounds.json保留同root顺序。该结果不追溯声称原 partial 下所有手势都不能访问。原真实 A12 只读窗口新回归在前置“恰两条completed reasoning”处失败，当前尚未到新卡片像素断言，需核新 JNI 当前真实 projection 与原 Run journal 的事实差异，未重模型/改写旧数据。

共享专家卡片动作主审新增缺口：Mac 原 `deliverUIInteraction` 会将 queued action 交通用 `send`；尚无证据证明该新 Run 具备完整 manifest 原模型/模板见证、expert continuation relation 和原 parent 下的显示关联。不能只因 Surface context ID 正确就开放并记动作完成。Apple 当前源码批改为专家展示真实 Core readonly revision、普通卡片保原交互，不在 Renderer 伪改 phase/删 actions；这是临时呈现门禁，**完整专家表单交互仍是本目标未完成项**，不得用只读替代最终能力。Shared `AppleMessagePartFlow.isExpanded` 已新增并供两 Apple renderer 复用，统一显式 false 优先，仍是客户端瞬时阅读状态，不进入 Core Run 权威。


### 真实只读回归暴露共享投影次序错误

Android actual-window-r3 的失败已纠正定位：并非“两条 reasoning 不符”；当前编译源码45行为该前置（已通过），46行实际是 Surface 必须早于最终非空 Text。`android-actual-projection-query-r1.log` 的独立只读 query **1/1通过，0.391秒**，`android-actual-source-projection-r1.json` 证明新 JNI 公开投影是 `R6 → Tool760 → R19 → Text18 → UiSurface`，而同一原 Run journal 是 `R6 → Tool760 → UiSurface → R19 → Text18 → empty Text5`。两条 reasoning 仍382/242B，正文仍原Text18的39B，没有转移到placeholder。Store readonly quick_check通过，未写数据/发送新请求。

实际根因已定位 `core/nola-application/src/conversation_projection.rs` 原690–692：checkpoint投影之后 `extend` 全部 Application-owned Surface。故上文“Core顺序正确”的结论仅适用于该 Run journal，不涵盖公开会话投影；四端都需共享 producer 修复。最初 root 锚点合并草稿尚未编译，静审进一步发现合法 checkpoint rebase 可移除原 Tool、公开32K预算也可将 SurfaceRef 延迟；不得将这些正常历史状态误拒为坏 Store。已由熟悉这两处端口的代理继续整批修复，必须沿可信 Store origin/owner/ordinal及现有有界历史读取合并，不客户端排序、不 tail append、不提升 Deferred 输出、不改写既有会话。源与真实 Store 回归待主审后，才统一新 Core / 原生桥编测。

Web Error Part 另按当前 Domain `code/display_key/recoverable` 修正：已知译文呈现人类标题、未知 key 使用7语言通用步骤失败；协议标识与可恢复标志保留在默认折叠技术详情，不从 recoverable 发起重试。原只含 `failure` 的 canonical 错误夹具改为当前真实契约（journal failure仍属另一来源），转义/折叠/无假retry/7语言均验。`web-canonical-error-tests.log` 受影响26/26，`web-canonical-error-check.log`0错误/0警告。不是新增真实Chrome点击证据。


### 已保存部件与 TaskControl 当前类型收口

Web `ChatJournalDetails` 现在复用当前 journal 与 canonical 两种公开投影，已保存的 Task / Skill / ModelAttempt / Compaction 等也在原 Part 锚点显示。终态隐藏旧 current_step/progress；Skill、模型 provider/ID、结果引用等技术事实进入默认折叠详情；明确 successor attempt 关系用中性重新请求标题，只有 fallback_model 字符串不推断重试。原 Task 与同 ID 的 TaskControl 不再重复主摘要。

`assistantJournal` 不再把当前 `task_control` 改名 `task`。普通 Task 即使出现同名 capability 字段也不能获得操作；TaskControl 仍须 exact journal owner 与显式 capability，并复用原持久命令/回执协议。新的协议 reader 测试经过真实 reader 的公开 wire 形状，不只用 reducer fixture。首次受影响48项中47通过、1个旧 summary selector 未匹配 Svelte scope class；保留失败后只修观察器并补 summary 数量断言，避免空集合断言虚假通过。第二次48/48，`web-canonical-metadata-check-r1.log`0错误/0警告；随后 `web-canonical-metadata-full-tests.log` **575/575、0跳过、9.499秒**。这是前端单元/协议/SSR套件，不等于新 Chrome 产品点击或真实模型闭环。

Root 后续审出 iOS 工具 reader 的实际 modal 仍挂在 Tool 子 View，单纯把 snapshot 外置不足以在 segment 重组时保住窗口。现 `ChatBubble` 改为消息层 fullScreenCover、单个明确 Part/消息快照、UI激活与Part集合门禁，旧 cover dismissal 按snapshot UUID不能清新窗口；子卡只发起打开，不再呈现外部窗口。syntax parse / diffcheck通过，**尚未编测或真实GUI验证**。Android reader同样补纯UI activation fence，A→B→A旧回调不重新打开/关闭新窗口，source/tests尚未编。Mac已要求核同样的modal宿主问题；新native编测继续等完整源码审计及共享Core producer修复。

### Chrome 恢复后的产品点击与统一编测启动

Chrome 已可绑定真实窗口（5187，隔离 web-real Host）。实际点击既有普通/专家会话、新建会话、输入 `/`、选择 `/model`、点击发送打开会话设置、关闭返回；设置面板有独立焦点/关闭入口，背景不可访问。候选点击只补全命令，执行后打开设置，不发送模型请求。新建会话实际 ID `conv-20261029211426441-1`；随后粘贴 `Reply only CHROME_REAL_CLICK_OK` 并点击发送，UI 已显示该用户消息、正在运行和停止入口。发送后的截图工具返回一次 ScreenCaptureKit -3811；再次只读 AX 确认已提交，没有重复发送。该模型任务仍待终态，不记成功。

切换会话期间观察到 URL 先变、旧内容暂留且输入禁用；重载/后续观察后能显示新 owner，尚未证实永久加载失败或具体根因。不能把此暂态记为通过，也不能误报已证明跨 owner 写入。真实专家旧会话 `c8521f8f-e6b1-426d-9ea8-b42d0f4c9a48` 显示两份 `WEB_REAL_EXPERT_EDIT_OK`，却仍“正在处理”且编辑禁用。只读 Store 诊断确认两个 child Run 已 sealed/answered，而 Room receipt 仍 executing；Host exact submission 查询漏掉既有批量查询的 Core ObserveExecution reconciliation。正在复用该端口补齐精确读，不用正文推断完成、不重新 dispatch 或直接改 Store。

最新完整前端检查 `web-final-scope-check.log` 0错误/0警告，`web-final-scope-tests.log` **576/576、0跳过、12.089秒**，生产构建通过（原有大 chunk 提示）。包含严格 canonical Error 契约和 task lineage 异步 owner/abort 门禁；与上述 Chrome 点击证据分别统计。

完整 renderer 源码主审已冻结，Core focused 编测启动。projection 首轮12通过/1夹具非法 origin.kind失败；改合法 builtin 后 exact 1/1，累计13个独立用例通过。真实 Store E2E 首轮5/7；新增 ordinary fixture admission revision/parent 不符、rebase 后空文本必须 Historical 的过强断言及其后续结果均保留日志，仍在按真实契约核查。原生新 archive/JNI 和真实 reader GUI 继续等待共享 Core gate，不以旧二进制声明新源码通过。

Chrome 新普通对话任务随后完成：实际可见 `CHROME_REAL_CLICK_OK`，同隔离 Store 只读核唯一 Run `f4680732-6251-49eb-8151-db913d29302f` 为 sealed，run_results.value.kind=answered。未切换原模型、未关闭工具、未重试该模型请求。终态时新用户消息只有复制/选中、没有编辑/分支入口；页面重载后出现“编辑并重发”，证实实时终态缺少 canonical revision 见证的呈现刷新缺口（不通过伪造 revision 开放操作）。已实际打开该编辑框，将内容改为 `Reply only CHROME_REAL_EDIT_OK` 并点击保存重发；后续结果待核。与之前只跑 API 的真实任务分开记录。

编辑重发后真实 UI 显示 `CHROME_REAL_EDIT_OK`，独立目标会话 `1af153ad-dd8e-4adc-83b9-9257782de375` 的唯一 Run `696ac071-3aae-4249-a8dd-0fd140704ad0` sealed/answered；两 Run invocation 的 model / tools / context_window / host_catalog 全量字段比较相等（不输出凭据）。原会话保留。随后实际输入 `@`，点击专家候选，输入栏只出现一个所选专家 chip，再点击移除；目录里同名项为不同历史 QA 定义，不以名称去重。概览、审阅、产物、进度、目标及关闭逐项实际点击：产物/进度/目标空状态和切换可见；审阅仍提示来源工作区不可用，未记完整审阅能力通过。

Core 最终受影响整批 `nola-projection-weave-focused-final.log` **14/14、0跳过**（真实 genUI7 / checkpoint归属2 / historical port2 / historical budget3）。Surface 案实际 BudgetDeferred、真实空 Rebase、cold reopen、退休 Tool/Reasoning readonly 均保留硬断言；空 Stored Text 保当前合法 Text 类型，不强制包装 Historical。ordinary archive 无 turn 绑定时，typed 字段按当前契约 Unsupported，独立 canonical_text 保 approved 和有界 preview；验证 total_utf8_bytes 精确、preview 少于原文、尾部未公开、候选隐私及原序，未伪造绑定。投影单元13个独立用例累计通过。原生新产物已获准按磁盘预算串行更新；此刻未有新 GUI 测试结果。

实时编辑入口缺口源码定位：`verifyTerminal` 只合并 admission/assistant journal；用户 canonical revision 仍为空，reload 的现 `loadHistory` 才补真实见证。计划复用既有 history 路径做终态后单次有 owner 门禁的读取；须处理正在读历史、重复 seal、异步 owner 换代、失败不撤销终态，以及已 prepend 长历史窗口/阅读锚点，不能直接后台 replace 丢旧窗口后宣称完整修复。该补齐尚未实施。

精确专家 receipt 观察源码已主审、CLI `room_http_contract_tests` **5/5、0跳过**，新 nola / nola-agent-host 构建通过。确认隔离 QA 的23 Run均sealed后，才正常停旧5187 Host并以相同DB/project启动新binary；不直接改数据库。Chrome原专家会话回看仍未通过：实际开发者控制台显示 manifest 查询409，页面专家历史不可读。只读诊断证实原 QA 全库缺当前 template_capture fact，Core准确拒绝 `expert_template_capture_witness_missing`，不是browser client差异，也未到exact receipt调用。不得给旧测试Store补写见证或降门禁；需当前正常admit形成的新source继续验证。新会话 `conv-20261029213452730-1` 已经实际创建/选专家，尚未提交模型。

独立 Chrome 组件交互页实际打开原 Tool / 技术详情，前插 Tool 后原 Tool 仍展开，完成终态后同样保原 Tool 展开、新 Tool关闭。技术详情自身在重组后收起，属于仍存在的更深阅读状态缺口；不是 Core 前插/真实模型证明。不能将“过程与工具主详情保留”扩大成全部嵌套 reader 已保留。

Mac r34 先在巨大 ViewBuilder 处 typechecker 超时，未运行测试；主审纯表达式拆分后，r35 全 app/test target 编译通过，真实选定测试启动但最终有4项失败（interleaved journal、sealed历史Task、两项真实NSWindow阅读）。错误日志与xcresult保留，正在分类；不能以编译成功或旧27项替代这批GUI通过。原生队列/磁盘约3GiB继续受控。

当前协议双专家 Chrome 闭环已完成：实际 `@` 选 Real expert1/2、提交 `CHROME_TWO_EXPERTS_OK`，两独立回复均可见且显示“已完成”，编辑入口恢复、原恢复按钮消失。独立只读核两 child `b1950c70-54d0-46f6-b885-c17264fe64d2` / `55288c03-d26a-4e18-bed9-2c74b84d41ae` 均 sealed/answered；对应 Room receipts 均 completed/revision4/terminal answered。capture fact1，全库Run从23增至25，只有两原admit child，没有 reconciliation 新Run。`chrome-two-experts-current.json` 留摘要，root已目视真实截图。原无capture样例409保留，未补写旧数据。随后已实际点击该新消息编辑并重发、改为 `CHROME_TWO_EXPERTS_EDIT_OK` 并提交，第二阶段尚待验收。

Mac r35准确结果73/77。两NSWindow缺Tool toggle的实际Button AX identifier，原标识挂在被DisclosureGroupStyle吞掉的label；窄修将既有语义ID挂真实Button，测试仍真实AX点击。终态整消息比live多12px不能判断过程折叠，改直接核实际process Toggle公开本地化Collapsed值，保其它真实展开/缩起断言。sealed历史Task则是共享Store真实缺陷：`semantic_part_draft::validate_enums` 固定状态白名单漏当前合法 sealed/waiting_for_input，导致typed历史Unsupported。准备按schema复用Domain当前枚举，未知仍拒绝；没有把fixture换completed。新源仍待iOS正在编的Rust archive完成再落，防止中途改在编源。

当前协议双专家编辑 Chrome 闭环亦完成：新目标 `884aca94-12cf-432b-9d13-7066894030b3` 两份 `CHROME_TWO_EXPERTS_EDIT_OK` 与已完成状态实际可见。独立只读报告 `web-current-expert-edit-readonly-report.json` 确认新 children `09cf03b1-ebd4-4ef7-97ad-cc0d91ab789f` / `4dc434ff-1653-45cc-bfd4-06c911a7048d` receipt均completed/revision3/answered，actual Run result answered、originating child精确。按同agent比较原model_declaration、FrozenModel、typed prompt_definitions、expected_prompt_definition全部相同；Room/context/Run/user/assistant为全新owner，原source/receipt仍在。整个witness envelope不应相等：fresh invocation/checkpoint/component digest正常不同，不能把它们冒充冻结模型差异。全库27Run，相对前25仅多预期2，无额外Run。

iOS新sim release archive实际构建10分33秒完成；Swift首轮历史详情request为optional的2处类型错误，经主审改有效if-let后production Nola target已完整编过。下一轮新增GUI fixture使用只读环境key作override导致测试源码编译错误，删该测试修饰、沿真实默认动画继续同DD增量；没有改生产动画或绕过AX action。各失败bundle均保留，新的reader GUI还未得到运行通过结果。共享Store枚举patch先存/tmp proposal且已主审，等本轮Swift/GUI结束再落工作区，避免在依赖构建中修改Core。

后续准确收口：iOS retry2 **45/48、0跳过**，公开UIKit AX枚举无法发现首个SwiftUI process按钮，未打开reader，不能算驻留反证或成功；另外两旧hosted夹具未注入ServiceContainer而崩溃，已改每测试独立Store/容器。无效app-host reader测试已删除，迁为DEBUG隔离展示fixture+真正XCUITest tap：cover前Arm、实际开reader后才变typed输入、同UIKit controller连续核对，Close后核status、重开新正文、换owner自动dismiss，明确不是Core/模型执行证据。新源经root完整审查，未私有反射、未直调Binding、未点modal遮住的按钮。

共享状态patch已应用且**23/23、零跳过**：Domain2、semantic draft6、historical projection9、Engine parts6。ToolPartState/StepStatus原定义下沉Domain唯一实现，Engine正常重导出当前API，无wire变更/兼容转换；Store逐schema复用真实枚举，Step仍不能接sealed。Android另补canonical Task等待输入的人类badge并停转圈，Tool/Task sealed七语言主文统一“执行已结束”；新增回归走真实MessageBubble Tool/Task，不直接画独立徽标冒充Tool验证。

磁盘曾仅1.5GiB；root清理target/debug/deps中今天以前的360个重复旧哈希可执行产物约21.71GiB（保留各target最新版、今天全部、所有库/当前程序/测试报告/DB，排除lsof仍使用的文件），`obsolete-rust-executables-cleanup.json`留清单，恢复约24GiB。全批源码再次冻结、root终审后放行：iOS精确2旧hosted方法+1新UITest，Android一次新JNI+focused13 JVM/20 Compose+原A12只读1，Mac后续仅失败4+typedsealed1。此刻构建仍进行，不能把计划数当通过数。


### 阅读窗口最新实际验收与剩余边界

iOS 最新 Core archive 包含原序 weave 与当前状态枚举。两条原 hosted 回归实际 **2/2**，新 XCUITest reader **1/1、零跳过、53.401 秒**，结果 `reader-container-scope-r3.xcresult`。真实点击过程、Tool、技术详情、阅读原文，四张附件显示原 A、前插更新后同一 A、关闭重开 B、换 owner 后自动关闭；连续采样同 UIKit presented controller。首次 UITest 将整个 app 背后 B 误算 modal 文本，精确限定实际 reader 后保留 A 可点/B 不在 modal/同 controller/owner 原断言才通过。root 已目视四图。这是生产组件真实 UI、typed 公开输入展示夹具，不宣称该夹具执行了 Core 或模型。

Android 最新 JNI 构建通过，focused JVM **13/13**；Compose 首轮 **19/20**，新 sealed Tool 用例在无 Scroll action 的根 Column 上误用 performScrollTo，尚未实际点击即失败。只移除该无效滚动，保留显示、真实点击、Tool neutral 色及 Task waiting 无 spinner 后，重跑 **1/1**；20 个不同用例均有通过证据。原 A12 同 Store 只读实际窗口 **1/1、13.473 秒**，原 Run `f37643a0-69a3-4108-bcdb-675dafc0e6fa`，未新模型/改数据。root 已独立目视整张 context/card 窗口，卡片在后续 reasoning 和最终正文前，原两段 completed reasoning 折叠。`android-reader-weave-actual-window-r1-artifacts` 保留 PNG、metadata、设备与拉取 SHA。不是原 20 行长模型失败任务的替代验收。

Mac r36 focused5中3通过（真实 Store sealed Task、typed sealed 与交错过程），两 reader GUI仍失败；r37真实 AX 树证实 Tool/Technical 是 AXDisclosureTriangle，reader open 是 AXLink，不能伪装为 Button。r38正确真实 Tool press 后推进，但精确 Technical ID仍缺失：外层 Tool identifier 继承覆盖全部后代。正按已有 process 容器方式修 children contain 隔离，保持实际控件/状态/窗口/owner 断言。**两 reader 此刻未通过**，不计 Mac 整批完成。

Web 阅读状态已集中为消息所属的纯 UI scope：精确 owner/Part/激活代际，嵌套技术详情、输入、Tool、过程显式选择；移除 Part 清理，旧回调及 owner/Part ABA拒绝。modal 宿主移到消息层，冻结已公开字符串快照，不增私有/全文资源读取端口。独立静审发现并修掉焦点回调无代际、裸 ID缓存不清理及子组件本地状态跨 owner 回流三处问题。

真实 Chrome 首轮又发现重要问题：前插后窗口被重建且 Back 无法关闭。`web-reader-chrome-r1-failure.txt` 保留失败；Svelte 深代理让冻结 snapshot 被重新代理，破坏 keyed modal 身份和 close CAS。生产改用 `$state.raw` 保留确切快照身份后，再真实点击完整链：A阅读中前插/内容B更新、连续8次同 DOM dialog、A不变；Back正常关闭，技术详情保留并恢复当前 opener；重开实际显示B；再阅读时换owner自动关闭，状态 `verified-owner-dismissed-and-focus-safe`。root 实际目视/AX核对，不是脚本调用点击，也不是模型任务。

最终生产 Web 源检查 **零错误/零警告**，全测试 **580/580、零跳过、12.732 秒**，生产 build exit0（仍有大于500KB chunk提醒），交互页 build exit0。日志 `web-message-reading-{check,full,production,interaction}-final.log`。新增内容隐藏/重新激活实际点击验证尚待执行，纯 scope 回归已有覆盖；不能把这条剩余 DOM 验证记作完成。普通真实任务终态后的 user canonical witness 刷新仍未实施；专家 own-source action continuation、完整历史 decoded range及总体管理/长任务矩阵仍属原目标，不由本轮 reader绿灯关闭。


Web 隐藏/重激活 DOM 验收已实际补齐：Chrome 在生产 reader 打开后由展示夹具切 active=false，真实 modal 自动移除，状态 `verified-inactive-dismissed`；点 Show content again 后 `verified-reactivated-without-dialog`，原 Tool/technical 均仍展开，没有自动重开；再点当前 reader 可正常开 A，Back 后 AX焦点精确回到当前 reader按钮。root 目视了重激活整窗。该夹具只变展示输入，不创建 Run。新增控制后的 check 仍零错误/零警告，interaction build exit0。随后完整5187产品实际reload，原合法双专家已完成答复/编辑入口仍可见；实际展开和收起第一专家公开思考正文正常，未重新发模型。

Mac r39 contain 后两 reader 仍失败于 Tool精确同节点AX角色；需要依据新实际树检验容器与真正 disclosure 控件的关系，不将contain源码或编译算作GUI修复通过。继续保留失败。整体goal仍进行中。


### 下一独立源码批提案：终态用户 canonical witness 补读（只读审计，尚未实施）

本段只登记审计与候选方案，不修改当前接口、模型或协议字段，也不将问题记为已修。后续实施前仍须独立审查源 snapshot 一致性；本轮 Web reader 的580项与真实 Chrome阅读通过不能覆盖这一问题。

**实际缺口。** `web/frontend/src/lib/chatTimeline.ts` 的 `admittedMessages` 为新用户行安装真实 admission 文本和 completed 展示状态，但 `canonical` 仍是 `old?.canonical ?? null`；`terminal` reducer 只按完整终态证据更新助手状态，不产生用户 Message/Part revision，也不刷新会话 projection revision。`web/frontend/src/lib/conversationSubmissionController.ts:416` 的 `verifyTerminal` 当前只读取已验证 seal/manifest/assistant journal 并正常释放 Run，没有随后读取用户 canonical 元数据。因此新会话真实终态之后，用户行可能仍无编辑所需的原始见证，直到显式历史 replace/reload。`web/frontend/src/lib/editedResend.ts` 的 `editedResendSourceForMessage` 拒绝该行是正确守卫，不应放宽为 optimistic、revision=0、admission sequence 或 terminal Core revision。

已有读取来源足够：`cli/src/web_parts/routes.rs` 的 `list_messages_response`（约2454–2468）返回真实 `sourceHostId`、`conversationProjection` 和同快照 `messageWindow.projectionRevision/eventSequence`。读取 canonical 用户 Message/Part revision 需要这类独立只读事实；当前终态/admission事件没有等价字段，不能本地推导。该 GET 不增加模型请求，也不重发任何 Run。

**待审的最小方案。** 终态被原完整 manifest 验证并提交之后，独立合并“确切用户行的读取见证”，不调用 `loadHistory('replace')`，不以历史读取成功作为终态成功条件。只对本次 admission 中真实 user ID、当前仍存在且确实缺少见证的 exact 用户行，采用响应中的真实 canonical 对象及其所属会话 snapshot revision；现有可见 Parts/正文、助手 journal、消息顺序、历史窗口、游标、durable 标志、submission parent 和 Run 终态全部保持。读取见证应与该 Message 一起保存在已有缓存内，具体表达由下一批源码审定；不得为了编辑临时把新 revision 拼到旧分页 before/eventSequence 上，或把旧 projection 元数据冒充新快照。当前末48条响应未包含的老用户行继续明确无见证；如后续决定扩读取，必须沿同一真实 snapshot 分页，不能把未读完整范围当完整结果。

补读的控制与归属必须同时满足：

- 捕获 immutable controller 的 Host/client/conversation/generation，以及本次 submission、Run、seal 和 timeline request version；响应后逐项核当前 owner、相同已提交终态、原行 identity 与 CAS。下一次提交、分页/历史替换、dispose 或 A→B→A 新 controller 不能消费旧响应。
- 严格核 `sourceHostId`、projection conversation、projection/window revision 一致，且真实会话/Message/Part revision 不倒退。只接受仍与已显示原用户内容和 role 相符的实际记录；不根据文本相似性选择另一个 Message，不从当前选中模型或客户端计数补字段。无记录、坏记录或不一致时保持编辑不可用。
- 单一只读补读使用独立请求票据/AbortController，并按 exact Run+seal 合并重复通知；不停止正常 observer，不抢占原 history 请求或写它的 loading/finally。失败只保留见证不可用的展示状态，不把已经确认的 terminal 改为 uncertain/running，不重新获取模型结果，不错误清理用户后输入草稿。
- 长历史已加载范围和原分页见证保留，不允许后台 replace 丢回末48条。若编辑入口新增造成行高变化，应在合并前最后一刻沿现 presentation `captureHistory`/`restoreHistory` 保存锚点，回写后仍核 selected controller 与 scroll token；不能把网络请求开始时的旧位置强行恢复到用户后来滚动的位置。旧分页 snapshot 失效的恢复另需保留范围审查，不在本方案里偷偷改游标或自动丢历史。

需要一起审查的三个实际 source 调用都在 `web/frontend/src/pages/Chat.svelte`：`refreshExpertSubmissions`（约562）、`resumeCompleteExpertMessage`（约948）、`ChatMessageActions` 的 `editSource` 属性（约1130）。三处当前都把 `timeline?.projection?.revision ?? 0` 传给 `editedResendSourceForMessage`；后续必须统一消费同一 Message 与其实际读取 snapshot 的配对见证，不能只修编辑按钮而让专家恢复继续使用旧会话 revision。`ChatMessageActions.svelte` 冻结 sourceKey/原草稿与 `editedResend.ts` 的真实源/版本门禁保持。

**下一批应有的行为边界（本轮未执行）。**

1. 新会话初始空历史→真实 admission/终态→只读补读后无需 reload 即取得确切 user revision 和原始 snapshot revision；记录仅 GET，无新模型/提交。
2. history/补读网络失败、无记录、sourceHost/role/body/版本错误：终态仍 committed、提交准入仍由既有事实决定、编辑继续不可用，原草稿不受影响。
3. 补读期间开始新 Run、prepend/replace 历史、切 Host/会话、A→B→A 或销毁 controller：旧结果与旧 finally 均不能写新状态；重复 terminal 通知不能并发重复补读。
4. 已展开超过48条的历史：所有已有行 ID、正文/Parts、journal cursor、窗口 before/eventSequence 与锚点不被后台补读替换；原可见 reader/显式披露选择不因补元数据销毁。
5. 相同 Message 已有更高 revision/读取快照时，旧响应不可降级；当前末页缺少目标行时不可造见证；包含附件或受保护/非 plain 的用户行仍按原编辑门禁拒绝。
6. 普通编辑入口、专家 manifest 读取/恢复与实际 prepared source CAS 都使用同一配对 source witness；不以“按钮出现”冒充重发准入或模型执行通过。


Mac r40 两条原失败真实 GUI 最终 **2/2、零失败、3.936 秒**，`ordered-parts-40.log/.xcresult`，Test/arm64/同 DerivedData 与最新共享 archive。r39 实际树明确语义 Tool ID位于 AXGroup，真正原生 disclosure 是其内部无 ID 的 AXDisclosureTriangle。测试按精确唯一语义容器→其受限公开子树内唯一真实 disclosure 定位；其它语义子容器停止、去环/2048上限、歧义拒绝，不任意角色降级。真实 press/Tool与technical展开/终态选择/同sheet/原source与Part ID/原4秒预算均保留。生产 contain修正确实隔离了子标识，原生 reader开入口严格AXLink，关闭严格AXButton。成功附件已从实际NSView及sheet.contentView生成并keepAlways，待root目视后记图片结论；不把全部77项重新运行过，当前是首轮73通过加修复项focused证据。


Root 已逐张目视 `reader-success-r40` 四张实际原生附件：live与terminal均保留同一Tool及技术详情展开，状态按各自输入切换；原始sheet与regroup后sheet均真实可见 `EXACT-READER-START`、长正文与关闭按钮，没有变成底层消息或空窗口。文件分别为 `DBD4507D-3686-4E15-81E3-DB6C9B1B8684.png`、`3947A24A-16FC-48CC-812A-D91B68A74B48.png`、`DA8AF209-2202-47FC-BC20-97E2ABBE2D89.png`、`036FAC99-4AB8-4E04-A63F-44E3A3286219.png`。同sheet身份仍由实际窗口断言证明，截图不替代owner/CAS断言；没有因此声称安全decoded全历史端口已实现。


### 终态用户见证补读：实施与首轮验证

已落新源：真实历史响应为每消息canonical配对source Host/会话/revision；独立terminal_witness只补完整verified seal声明的确切plain用户metadata。编辑及两处专家来源调用、专家刷新effect与manifest cache均改同一配对revision，不回退旧分页revision。controller独立GET，不走loadHistory；切走/后台/新Run/新history使票据失效，等待正在飞的history结束，正文/窗口/journal不替换。metadata锚点恢复在实际apply前捕获、tick后仍核原票据/presentation/terminal及scrolltoken；followLatest不被强改成reading。没有增加服务端协议或模型请求。

独立静审发现并收口在飞history被metadata version抢失效、按turn对象引用误拒同终态、tick后旧presentation可写三处竞态。首轮check只有新增test的TimelinePart联合类型构造错误，改精确canonical fixture分支后0错误/0警告。定向首轮40/41，history-in-flight夹具最初误以initialize会在待定history期间自动启动observer，实际须沿公开controller.observe触发独立authority读，未改生产启动顺序或预算。随后发现同Run重复admission会重建用户Parts数组，真实生产已改精确身份/正文匹配时保留原Parts；保留strict引用断言，测试以真正metadata apply边界核对。完整首轮591中588通过，3失败包括该in-flight观察及两处prepared-resend fixture漏当前sourceHost；补正确wire来源、修生产idempotence后定向4文件43/43、零跳过、2.921秒。失败日志均保留：web-terminal-witness-{check-r1,focused-r1,r2,r3,full-r1}.log。尚待最后源码审查、完整新suite/build和Chrome真实模型，不把定向通过算端到端完成。


终态见证最后审计补齐：同Message已有canonical但缺snapshot时，拒绝同ID Part revision倒退；重复admissions连续两次严格保留Parts/canonical/snapshot对象引用，optimistic同文仍由真实admission替换。最后全源冻结后 `web-terminal-witness-check-final.log` **0错误/0警告**，`web-terminal-witness-full-final.log` **593/593、零跳过、24.049秒**，production build **exit0、2.03秒**（保留>500KB chunk提醒）。独立审核确认三竞态收口，无新阻断；不是新服务端兼容层。

真实Chrome本轮仅到新建会话 `conv-20261029232339229-2`，未成功输入/发送任务。新buildreload后观察到旧专家正文暂未出现、URL切换后旧内容停留；打开实际DevTools时新会话才可见且console0条错误，未有足够证据区分页面延迟与桌面/捕获环境异常，不记永久产品故障或通过。随后输入AX/paste/type均无Value变化，坐标操作返回 `noWindowsAvailable`，盘点Chrome进程仍在但所有应用无窗口、browser列表为空。没有重发/改模型/延长deadline。真实终态后无需refresh编辑重发这一最终验收仍未执行；已请求恢复桌面窗口。不能以593绿灯或前一轮6个模型Run替代当前改动真实验证。
