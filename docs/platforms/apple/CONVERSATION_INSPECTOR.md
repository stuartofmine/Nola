> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Apple 会话检查面板

核对：2026-11-08，本地工作树。共同语义见[会话检查器](../../design/shared/CONVERSATION_INSPECTOR.md)，布局规范不能从旧原型切页行为反推。

## macOS

`MacChatInspector` 持有同一 `MacInspectorNavigationState`。`expandedSections` 表示独立展开集合；`selectedTab` 是现有代码中的定位目标，不再表示只渲染一个页面。`revealRevision` 让重复定位同一区段也能重新展开并滚动。`expandedKnowledge` 分别保存知识点、Wiki、召回的展开状态。

面板使用固定标题、关闭按钮、一个主要 ScrollView 和原地 DisclosureGroup。窄覆盖层可复用顶栏工具入口，宽侧栏不再另建页选择器；选择工具不替换消息或中央标签。展开动画遵循系统减少动态效果设置。

提示词模板入口已从检查器上下文区移入 `MacConversationModelSettingsView`，与会话模型编辑器的保存/取消共用草稿周期；模板库仍是独立管理入口。模板选择仍写会话配置，不因入口移动而变成全局模型默认值。

## iOS

手机以原生导航和面板适配，不照搬桌面三栏。`IOSConversationInspector` 的资源详情使用原地 DisclosureGroup。`ChatMoreSettingsSheet` 的模型区提供提示词模板选择，成功提交后才更新当前选择；上下文面板不再重复放置模板控件。

紧凑聊天标题隐藏系统导航栏时，`ChatNavigationGestureBridge` 恢复 UINavigationController 的原生边缘返回。只在聊天可见且可返回时接管 delegate；交互返回进行中不能提前禁用 recognizer，完成离开后恢复原 delegate/启用状态，取消返回保持当前聊天与草稿。根页面和已呈现模态不能被当作可返回聊天。

## 数据与验证

进度仍读取当前 owner 的 Core session projection 与 conversation plan；superseded 工作不显示为当前工作，reported_done 不冒充完成。未报告数据保持未知。请求失败保留本 owner 最近成功内容，切 owner 不展示前一个会话，所有迟到结果按原身份处理。

视图挂载、展开、可见性和业务观察是不同生命周期；不能将旧“切页即停止所有读取”的描述沿用到可同时展开的面板。共享配置、流式正文与读取边界见[客户端交互与配置](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。

回归入口：`MacInspectorInteractionTests`、`ChatNavigationGestureTests`、`ChatComposerLayoutTests`。组件、隔离存储和模拟器证据不等于真实模型、真机全流程或性能验收。
