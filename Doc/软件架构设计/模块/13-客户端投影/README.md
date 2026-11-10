> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-13：客户端投影与 UI

[A2UI Runtime 与客户端投影](A2UI_RUNTIME_CONTRACT.md)：固定协议、Surface 所有权、事务/增量、动作和第三方渲染适配；现有实现已接共享 Core，实际平台动态覆盖另列。

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 输入：Application 查询 snapshot + committed cursor 增量。输出：用户命令与纯展示状态。
- 展示状态可含滚动锚点、展开、选中 tab、未提交草稿；运行/授权/缓存 usage 不由 ViewModel 独立决定。
- 订阅 key 至少包含 conversation_id/run_id/generation。切换会话后旧回调不能控制新输入框。
- 正文按稳定 Message/Part identity 增量渲染，完成仅 seal；不能以替换全文制造闪烁或滚动跳转。
- 工具名和状态本地化由 schema presentation key 映射，不翻译 ID，不从文字决定控制状态。
- 验证：Mac、CLI、iOS、Android、Web 各自真实操作；共用 reducer 测试不能证明各端布局/线程正确。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

会话数据仓库、会话草稿、Run 观察器、窗口展示状态分离；ViewModel 负责装配而非业务裁决。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [lib.rs](../../../../core/nola-mobile-bindings/src/lib.rs) | 最终原生产物、Application C ABI、embedded 句柄 |
| [projection.rs](../../../../core/nola-mobile-bindings/src/projection.rs) | 移动投影与读取桥接 |
| [RemoteSubmissionIdentity.swift](../../../../Shared/Protocol/RemoteSubmissionIdentity.swift) | Apple 源准入身份共享 DTO |

原生静态/动态库由 mobile_bindings 产出，再保留 Core 辅助符号。Swift/Kotlin 的观察、草稿、IME 与焦点不是第二个业务 Runtime；手机导航不改变已准入 Run。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [客户端交互与配置当前接线](CLIENT_INTERACTION_AND_CONFIGURATION.md)：原生档位、模板作用域、检查器展开、移动手势与验证边界（2026-11-08）。

- [CLIENT_PROJECTION_BOUNDARY](CLIENT_PROJECTION_BOUNDARY.md)
- [CANONICAL_STREAMING_TEXT_CONTRACT](CANONICAL_STREAMING_TEXT_CONTRACT.md)
- [CLIENT_BOUNDARIES](CLIENT_BOUNDARIES.md)
- [MOBILE_SHARED_CORE](MOBILE_SHARED_CORE.md)
- [mobile-agent-architecture](mobile-agent-architecture.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 当前跨端消费边界（2026-10-28）

各端消费同一Core业务状态与A2UI v0.9.1投影；草稿、IME、焦点和滚动属于客户端。Apple共享submission frontier DTO；同一源记录的camelCase嵌套诊断不能被外层snake_case策略改写。

具体字段、返回值、错误和源码索引见[当前 Core 与 Host 接口](../01-领域与协议/CURRENT_RUNTIME_INTERFACES.md)。本次仅核对文档，不新增运行验收结论。


### 流式绘制边界（2026-11-10）

水面浮现是客户端绘制状态，不进入 Core、Run 或 transport 的完成条件。
Apple `AppleStreamingInlineText`、Android `MeasuredMarkdownTextView` 与 Web
`StreamingText` 立即保留全部 accepted 正文，仅新增 range 在 520ms 内由柔焦、
低透明度与下方 3 单位位移收敛至最终样式。上游完成后绘制尾部最多 160ms
收清；不得通过卸载组件或重建正文跳过收尾。减少动态效果、历史和修订立即显示。

活跃绘制片段有界，超限前缀立即沉入已清晰正文；动画不得阻塞保存、复制、
可访问性或后续 delta。相邻批次按内容顺序衔接，emoji/组合字符以 grapheme
为边界。原生代码块等结构化渲染仍沿用各平台既有测量与滚动路径。

### 帧合并与阅读所有权（2026-11-10）

客户端的跟随滚动由内容变化及实际布局高度变化共同唤醒。Android 使用有界
合并请求通道与单个帧消费者，后续 delta 延长短暂跟随窗口，不重启延迟任务；
每次滚动使用独立子任务，输入取消单次滚动不能取消后续跟随能力。手势开始
通过 DragInteraction 接管，以 NestedScrollConnection 的已消费滚动与
最终 fling 回调判断恢复；布局快照不能制造用户意图。
iOS 延续既有合并任务，将窗口拆为更细的显示节拍。此处不改变 Core、协议、
数据库或生成完成语义。Apple 新字绘制最多覆盖最后 384 字符，单个 run 内
最多三个 glyph 合并绘制，超出预算的前缀立即清晰且全文仍可复制、保存。

Web 路由完成焦点以导航代数和 DOM 挂载为边界，旧异步页面失效后不得抢焦点；
原生 Windows 启动画面与系统主题同步，窗口关闭时注销主题监听。上述都是
客户端呈现状态，不创建新的后台服务或数据兼容层。

### 设置导航与偏好所有权（2026-11-10）

设置分类是客户端导航状态。通用聚合基础偏好与平台启动/更新，不改变各偏好的
存储键、默认值、Core 所有权或系统授权接口。Mac 已保存的通知分类和 Web 的
通知直达地址解析到通用；Web 仍通过 workspacePageLocation/Elements 定位所属
页面，不能改写外层会话导航。Android 设置容器保存通用状态及可保存页面状态，
通知中心退出后恢复原页。内容/计划管理从其实际工作区进入，不复制管理实现。
