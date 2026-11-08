> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

> **单版本开发约束：仅保留当前协议与数据格式；不保留旧入口兼容，不迁移或自动改写用户数据。**

# 会话右侧检查面板实施

需求：标题栏最右侧三个小图标直接打开进度、资料、设置；进度展示当前会话的真实计划、待办和运行情况。

设计：[会话右侧检查面板](../../UI和交互设计/CONVERSATION_INSPECTOR.md)。本轮实施及定向验收已完成；以下记录仅覆盖本轮范围，不代表完整真实模型端到端验收。

## 输入与所有权

- Goal：`load_session_projection` / 既有Web `GET conversations/:id/goal`。读取目标和requirements条目，不只展示数量。
- Plan：现有Core `conversation_plan` 的 `get` 命令读取todos。普通会话不一定启用Goal，因此无Goal时仍必须读取Plan。
- 运行：当前会话精确owner的执行投影；不读取全局最近一次运行。
- UI：单一选中面板或关闭；切会话作废旧读取；主消息列表、草稿和订阅不随侧栏重挂载。

## 工作包

| 工作包 | 负责人 | 状态 |
| --- | --- | --- |
| Mac/iOS三入口及Goal/Plan | apple_ui_mainline | 已实施；iOS4项通过并检查390/834浅深空态截图；Mac首轮11项、最终实际工作区接线复验2项通过，主代理目检980窄窗口进度面板三入口和关闭按钮 |
| Web三入口与只读Plan接线 | account_gateway_core | 已实施：431项前端测试、Svelte零错误/警告、Rust路由1项及产品构建通过；独立Chrome四闭环通过，主代理已目检最终宽窄截图 |
| Android单sheet与精确会话读取 | other_clients_ui | 已实施：Kotlin/APK构建、JVM4项、Compose3项通过；本地Core读取，远程缺权威数据明确不可用，不伪造；非完整Chat端到端验收 |
| 总设计、跨端契约与结果审计 | 主代理 | 已完成设计、跨端所有权和最终证据核对；Web已部署，原生客户端需重新构建安装 |

## 验收约束

最终主题色复验7/7通过（`/tmp/nola-inspector-polish-r3-tests.log`），主代理再次检查深色长任务图，活动图标使用水墨青而非系统蓝。未覆盖运行中的用户Debug应用，需重新构建才能体验改动。

后续有内容状态打磨：Mac进度展示拆为独立Equatable内容组件，目标/执行/计划/要求/工作记录五区，24pt区段间距；保持计划顺序，未知与reported_done不显示完成勾。相同快照不重复赋值，新标题覆盖原有9语言。新增320pt长中文计划浅深色fixture截图并主代理目检；集中18/18通过，日志 `/tmp/nola-inspector-polish-r2-tests.log`。截图发现独立组件缺少主题继承，活动图标已改为直接复用MacDesignColor.brand；最终专项复验另记。切换面板仍会卸载进度视图，本轮不宣称实现跨面板缓存；性能录屏仍未验收。

2026-10-19 Mac细化：同owner刷新保留最近结果、失败原位提示；换owner清空隔离；轻量空态与待办状态图标；工作区正文仅在侧栏状态改变时隔离继承动画。Test配置集中验收16/16通过（进度5、交互10、宽窄浅深截图1），日志 `/tmp/nola-inspector-polish-tests.log`。主代理目检更新后的980浅色进度截图。未进行真实模型流式帧率或连续操作录屏验收，不能据此宣称卡顿全部解决；本次细化仅Mac。

不使用本地模型；5001现在使用用户正式库，禁止把它用作可写fixture。Web浏览器交互使用独立fixture/临时Host，Apple使用Test配置，Android仅隔离模拟器。须验证三入口互切/收起、Goal空而Plan非空、切会话拒绝迟到数据、已完成/受阻状态、宽窄布局和本地化。

Web最终截图与报告：`nola-inspector-ui.pn3MAw`。主代理确认正常canonical回答存在，宽屏面板为单层分区，窄屏单模态内可同图标互切；原错误fixture图不算正常页面证据。正式服务已以最新构建重启，保持同一正式库，当前会话只读Plan接口返回authoritative=true、plan=null，Chat HTTP200；没有向正式库写测试计划或调用模型。

Mac最终实际工作区证据：`/tmp/nola-conversation-inspector-macos-r5.log`（2/2），`/tmp/nola-inspector-progress-980-light.png`（主代理已目检）。宽侧栏和窄覆盖层复用同一 inspectorContent，避免重复实例遗漏导航参数。更多截图和各端验证见 UI_APPLE_AUDIT、UI_ANDROID_AUDIT、UI_WEB_AUDIT。

剩余边界：Android远程Host尚无权威Goal/Plan读取入口，明确显示不可用；iOS资料面板暂聚合上传文档，原消息附件及引用入口保留，尚非全部资料聚合。本轮没有调用真实模型、修改正式库测试数据或覆盖用户正在使用的签名Debug应用。
