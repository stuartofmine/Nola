> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 客户端交互、配置与调用边界

核对：2026-11-08，本地当前工作树。本文记录本次核对的模型选择、原生推理档位、提示词入口、检查器和移动导航，不将整个客户端或历史架构目标标记为完成。产品交互见[对话规范](../../../UI和交互设计/CONVERSATION_UI_DESIGN.md)与[检查器规范](../../../UI和交互设计/CONVERSATION_INSPECTOR.md)。

## 状态所有者

| 状态 | 所有者 | 展示层可做的操作 |
| --- | --- | --- |
| 连接、账户、目录、能力与模型默认参数 | Core/Application 模型仓库，凭据由 Host 私有存储解析 | 提交带身份与 revision 的配置命令；能力未知保持未知 |
| 当前会话模型选择与参数覆盖 | 会话配置、`modelSelectionID`/`modelOverrides` 与配置 revision | 绑定原 owner，提交稀疏 patch/CAS；失败保留草稿，不整包覆盖无关字段 |
| 已准入 Run 的参数与路线 | 准入冻结的调用快照 | 观察与控制原 Run；修改当前菜单不回写在途请求 |
| 模板内容与选中模板 | 模板服务拥有内容；会话拥有模板选择，成员快照遵循参与者契约 | 在模型设置中编辑选择；入口移动不改变存储作用域或凭据所有权 |
| 输入草稿、附件导入、滚动、焦点 | 客户端按 Host/会话隔离的草稿及展示对象 | 导航保留草稿；仅提交确认后按原草稿版本清理 |
| 检查器展开、定位目标与转场进度 | 客户端展示状态 | 原地展开、关闭、定位；不创建 Goal、不恢复或重放任务、不发出工具授权 |
| 消息、Part、任务状态、用量与缓存 | 已提交的 Core/Application 投影 | 消费事实，保留未知和失败；不从动画或文本推算业务终态 |

全局默认、会话覆盖、临时编辑草稿与冻结调用是不同对象。窗口切换、面板开合与后台执行拥有不同生命周期；异步结果不能读取“当前选中会话”来决定写入目标。

## 原生推理档位的调用链

1. 模型目录声明 `reasoning.control`、实际 `efforts`/`budgets`、执行能力和协议。客户端不得按名称、模型家族或旧固定三档制造声明。
2. 聊天 effort 控件直接投影声明列表，保存精确 `reasoningPreference`。五档保留五档，不映射成三档产品风格。
3. 模型默认参数与会话覆盖分别保存。未设置表示继承；稀疏 patch 中 `null` 删除覆盖；字符串 `automatic` 表示提供商默认。序列化不得把未设置自动写成 `automatic`。
4. 新的精确选择操作清除冲突 `reasoningTier`。`session_override_request` 将精确 effort 翻译为一个 `reasoning_selection` 并去掉粗粒度 tier；底层 agent/delegation 的显式 tier 意图仍是现行领域能力，不是另一个聊天菜单。
5. Application 通过 `resolve_remote_model_profile`/`resolve_model_reasoning` 校验所选 profile/revision、协议、能力、输出限制和精确选择。直接 profile 解析请求同时提供非空 selection 与 tier 时拒绝；不得静默忽略冲突。
6. 显式请求优先于模型默认。无请求覆盖时 resolver 可读取 profile 的精确默认偏好。`automatic` 与仍显式携带的 agent tier 的组合按当前 resolver 解析；不能凭 UI 标签推定 tier 已消失，因此新 UI 写入必须实际清除它。
7. Host/provider adapter 消费冻结 selection，按协议序列化。采样与公开 reasoning summary 是独立能力，不能从 effort 名称推断。修改配置不改写旧 Run；未知/未声明 effort 拒绝，不自动选择“最接近”一档。

具体 seed 三态、条件采样与公开摘要见[生成参数契约](../08-模型配置与调用/GENERATION_OVERRIDE_CONTRACT.md)。

## 当前平台范围

| 平台/路径 | 已核对接线 | 不可扩大宣称的范围 |
| --- | --- | --- |
| Mac 本机 Nola 路径 | `MacComposerView` 原生 effort 菜单；预算声明在 Mac 以实际 token 值呈现；`MacConversationModelOverride` 保存精确偏好；本机请求不再补 `balanced` tier | 连接 Host 的目录当前不提供同等声明给此控件，原生菜单隐藏；远程 intent 的既有 tier 合同由源 Host 准入，不能用本机目录猜测 |
| iOS | `ReasoningEffortMenu`、模型参数页、`ModelSelectionManager`、`IOSPreparedRunModel` 将 effort 保存、解析并传入请求；已有会话用稀疏 CAS patch，未发送草稿由草稿配置拥有 | 不再写成“仅支持 automatic”；本次不证明预算 UI、所有 provider 路线或真机后台执行已验收 |
| Android | `ReasoningEffortMenu`、`ChatViewModel`、`ModelParametersStore`、参数页与会话序列化保留精确 effort | 原生入口主要针对 effort 声明；不将 Core 支持 budget 等同于已提供同样的预算选择 UI |
| Web | `ChatReasoningMenu` 显示声明 effort，`chatSessionSettings` 提交会话 patch；删除与输入菜单重复的旧 quick select | budget 仍遵循当前 Core/会话控制合同，不用五档 effort 的测试证明预算交互；浏览器限制另行适配 |
| CLI | `modelSelectionID`/`modelOverrides`、Core 目录和 Host 准入继续决定路线 | 模型档位入口核对未改造 CLI 模型交互；本轮终端文本编辑改进见下节，不能据此关闭权限指引、图片粘贴或全部显示问题 |

## 检查器与观察

Mac 用 `expandedSections`/`expandedKnowledge` 保存独立展开集合，`selectedTab` 是现有定位字段，`revealRevision` 触发重复定位。Web 用独立展开集合和首次访问挂载集合，设置组件仍只有一个实例，为输入区提供当前模型/档位状态。

原地折叠不等于卸载，更不等于停止 Run。Web 已访问区段保留已挂载组件和草稿；首次展开前不启动该区段的进度组件。不得把旧“切页即取消所有轮询”的说明套在所有端。读取请求绑定原 owner/generation；同一 owner 更新失败保留最后成功值，新 owner 不显示旧结果。

Mac 模板选择位于会话模型编辑器，按保存/取消暂存；iOS 模型区选择即时提交。它们均不把模板复制成另一份全局模型默认，也不自动更改其他会话。

## 导航、动画与流式文本

iOS 紧凑宽度由 `MainTabView` 的自定义底部导航和 `IOSCompactTabContainer` 承载。容器按标签缓存 `UIHostingController`，只挂载当前根页面；切换时结束旧页编辑，保留已访问标签的导航栈和表单草稿。主题、字号、语言和排版方向显式传入新 hosting 根，键盘避让由外层 SwiftUI shell 负责。紧凑布局不创建再隐藏系统 `TabView`：iOS 27 的隐藏原生标签栏仍可留下无限匹配动画，导致 UI 自动化每次输入等待约 120 秒。常规宽度继续使用系统 `TabView` / sidebarAdaptable。

iOS 紧凑聊天使用 UIKit 原生 interactive pop；delegate 只在该聊天可见期间接管，返回过程中保持 recognizer，完成离开后恢复，取消保留原页面。Android 预测返回以 seekable transition 跟随手势，键盘/模态优先消费返回，取消复位且尊重减少动态效果；偏移方向适配 RTL。Web 抽屉分 start/end：手机边缘打开，标题区向外滑动关闭；短拖、纵滚、长按、可编辑区域与代码选择不误触。按钮和 Escape 仍可用，关闭恢复 opener 焦点。

iOS 的 Stop 从提交用例沿会话 owner 调用 execution。原生 coordinator 必须先持久化精确的 Core interrupt，再取消提交消费者或退役 UI 观察；反过来会被当成 observer detach，清除物理执行 permit 并留下 running Run。发送是否恢复仍以取消确认和 canonical 准入检查为准，流 EOF 不能替代终态封口。

动画只影响展示树。历史正文即时可读；流式正文保留稳定段落与活跃尾段，完成或工具事件不重播旧段落。Apple/Android 的段落与代码围栏投影保持同样的边界规则；持久文本仍由 canonical Part/field 拥有，详见[流式文本契约](CANONICAL_STREAMING_TEXT_CONTRACT.md)。未知 usage/cache-hit 是未知，不是 0%。

## 可复核源码与验证边界

- Core：[profile resolver](../../../../core/nola-application/src/model_profile_resolution.rs)、[reasoning resolver](../../../../core/nola-application/src/model_generation.rs)。
- Apple：[Mac 检查器](../../../../NolaMac/Features/Inspector/MacChatInspectorView.swift)、[Mac 模型设置](../../../../NolaMac/Features/Chat/MacConversationModelSettingsView.swift)、[iOS 模型解析](../../../../Nola/Services/ModelSelectionManager.swift)、[iOS 导航](../../../../Nola/UI/Chat/ChatNavigationGestureBridge.swift)、[紧凑标签宿主](../../../../Nola/UI/IOSCompactTabContainer.swift)。
- Android：[聊天](../../../../android/app/src/main/java/com/nola/ui/screen/chat/ChatScreen.kt)、[effort 菜单](../../../../android/app/src/main/java/com/nola/ui/screen/chat/ReasoningEffortMenu.kt)、[段落投影](../../../../android/app/src/main/java/com/nola/agent/trace/StreamingMarkdownProjection.kt)。
- Web：[聊天/检查器](../../../../web/frontend/src/pages/Chat.svelte)、[抽屉](../../../../web/frontend/src/components/ChatResponsivePanel.svelte)、[手势判定](../../../../web/frontend/src/lib/panelSwipe.ts)、[会话参数](../../../../web/frontend/src/lib/chatSessionSettings.ts)。

回归入口包括 `MacInspectorInteractionTests`、`MacReasoningConfigurationTests`、`ChatNavigationGestureTests`、`IOSCompactTabContainerTests`、`ReasoningEffortSelectionTests`、Android 的 `ReasoningEffortSelectionTest` 和 Web 的 `panelSwipe.test.ts`/`chatSessionSettings.test.ts`。隔离测试、模拟器和浏览器组件检查分别记录，不能推导整窗帧率、全量真机、真实 provider、远程 Host 或 CLI 的验收结论。

## 展示任务、焦点和宿主窗口（2026-11-08）

复制反馈是消息展示对象拥有的可取消任务。iOS、Mac、Web 的重复复制替换旧的 1.8 秒计时，所有者替换/销毁时清理，不能由旧异步回调清除新消息反馈。复制不写入领域消息或触发 Run。移动动作布局只改变投影密度与点击区域，不改变编辑重发的 owner/revision。

Web 的搜索焦点恢复受展开代次与当前可见会话约束，失效 opener 不恢复。跟随滚动合并到 requestAnimationFrame；视口 ResizeObserver 观察容器尺寸，用户滚动使旧跟随任务失效。该观察不等同于任意异步图片内容的尺寸观察，不改变会话持久化或取消语义。

Windows 窗口几何由宿主对当前 display.workArea 校验，纯函数裁剪完整窗口并保留最大化意图；无有效交叠的旧位置交由主屏居中。原生菜单处理编辑、缩放、全屏和退出，聊天业务仍由共享 Web/Application 承担。CLI 模态文本编辑复用组合字符/按词编辑函数；提交与授权仍由各自明确按键合同消费，不把重复字符输入扩展为重复批准。

验证边界和证据见[本轮人机工程记录](../../../当前任务/CROSS_PLATFORM_ERGONOMICS_20261108.md)。本轮 CLI 编辑器改动不代表终端图片粘贴及全部权限指引已完成验收。

第二轮键盘细节：Mac 文本视图在普通回车处理前过滤自动重复，IME 标记文本交还 AppKit；Android 文本视图仅在精确粘贴组合的首次 KeyDown 导入附件。Web 档位控件的焦点定位与选择分离，只有显式选择调用 `onselect`；菜单位移属于展示状态，根据可视区域重新计算，不写入会话配置。键盘交互及 RTL 窄屏组件验收见上述记录的第二轮章节。
