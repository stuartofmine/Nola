> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

# Android 全面 UI 审计与优化

范围：Compose/Material 3 的导航、Chat/Room、模型账户、设置、授权与预览。保持水墨青山语义色、系统字体、48dp触控目标；不移植Apple平台导航或玻璃效果，不改执行/授权权威。

设计依据：已读 apple-design 的 accessibility/layout/typography/color/designing-for-ios/cross-platform 与 settings/tab-bars/entering-data。仅采纳通用原则：系统字体缩放、空间适配、语义对比、导航保持、明确输入和即时反馈；Android遵循Material组件行为。

## 设计方向

安静的内容工作台：绿色只突出主要操作与选中状态，表面分层而非堆阴影；共享页面最大840dp，紧凑宽度留16dp边距，常规宽度保持现行间距。页面标题和副操作应在窄屏/大字体下换行，不能让副操作挤没标题。动效尊重系统减少动画，不为装饰增加动画。

## 初始证据与边界

- 真实完整MainActivity→Room已进入；截图 `/tmp/nola-android-room-current-r3.X8M7I9/room-full-mention.png` 可见结构化@选择与草稿。未发送消息/调用模型。最初测试query含空格不符合当前单token候选规则，已纠正测试；空TextField还包含placeholder语义，改精确断言EditableText，不改产品语义。
- 默认Runtime的no_backup为0771导致提交日志拒绝：已保留原DB路径，仅验证app UID/真实目录/父目录与打开fd一致后0700。安全测试实际通过；完整页面当前继续。
- 既有授权组件浅深/320dp/2倍字体及导航测试8/8，JVM owner/composer16/16；不等于真实模型授权Run验收。
- 代码已有NolaPage 840dp限宽、Material底部/侧导航、语义主题和减少动画工具，应保留。不为全面优化重新造框架。

## 待核与高影响项

1. 共享NolaPageHeader固定横排，含trailing时标题空间受挤压；核对实际调用与大字体组件证据后调整。
2. 宽侧导航宽度动画未消费reduce-motion；核对共享动效工具后修。
3. Settings行单行标题/截断说明、模型账户表单和Chat预览分别核对，只有关键内容不可达才改，不机械删除所有省略。

验证采用隔离模拟器与现有Compose/JVM测试；禁止真实账户、网络授权、模型与物理设备。测试失败保留，截图须实际目检。

## 实施

- 模型页页面标题提前到连接库存之前，先说明当前位置再呈现账户/配置内容；账户管理仍是授权Host handoff，不改变来源登录/推理边界。
- NolaPageHeader在内容宽度小于360dp或字体倍率至少1.5时将副操作放至下一行右侧，避免挤压标题与说明；保持Material按钮和原行为。共享影响Data/Models/Settings。
- SettingsInfoRow不再把只读值截断为1/2行；长状态/版本信息可完整换行。可复制标识行继续保留原复制入口，不机械展开全部ID。
- 宽导航的展开宽度动画接既有reduce-motion设置，关闭系统动画时立即切换，不增加装饰动效。
- Chat/Room保持原执行语义、工具默认折叠与独立授权卡；ModelParametersSheet既有LazyColumn/键盘适配、权限48dp操作和截图预览原授权边界保留，未发现需要改执行架构的问题。

## 真实完整页面结果

最新arm64 Core/JNI与APK构建成功（5m52s，`/tmp/nola-android-room-current-core-build.log`）。全新隔离模拟器真实完整页面+安全目录 **2/2通过**（8.204s），证据 `/tmp/nola-android-room-current-r4.J7ICRR/instrumentation.log`。实际断言：进入MainActivity、自动收件人、@候选选择、A草稿→B空草稿/自动收件人→A原草稿及收件人恢复。截图同目录`room-full-candidates.png`、`room-full-mention.png`、`room-full-other-owner.png`；已目检，无越界，发送禁用状态明确。

这验证真实本地Core/VM/完整Room页面，不是模型发送或真实授权Run验收。没有用户数据删除或账户访问。

## 最终验证与目检

- UI APK最终构建成功1m5s：`/tmp/nola-android-ui-audit-final-build.log`；组件回归12/12（36.212s）：`/tmp/nola-android-ui-audit-final-instrumentation.log`。包括大字体标题/操作分离、设置完整值、模型compact/wide/Host管理、设置通知入口、Room浅深色及授权2倍字体。
- JVM Room composer/owner/repository定向16/16，Gradle成功45s：`/tmp/nola-android-ui-audit-final-jvm.log`。
- 实际全屏模型页发现库存说明占满首屏：进一步将API/Host连接和原紧邻反馈放在库存之前。最终`models-page-final.png`首屏两种连接入口完整可见；`settings-page.png`层级清楚。新`adaptive-header-large.png`、`settings-value-large.png`已目检完整换行，无裁切；均在`/tmp/nola-android-room-current-r4.J7ICRR/`。
- `room-full-mention.png`是Compose root捕获，系统IME窗口未绘入导致空白。另真实adb全屏`room-ime-visible.png`证明composer贴键盘；返回键隐藏IME后的`room-ime-hidden.png`证明composer回到底部tab上方，无残余空白。不要把root截图当键盘已隐藏。
- 仅本轮owned emulator-5580已关闭，进程exit0；原fixture与日志保留。未测真实模型/远端授权/物理设备。无需为此继续扩展fixture框架。

## 会话检查面板（新增需求）

遵循总设计`Doc/UI和交互设计/CONVERSATION_INSPECTOR.md`。Chat标题最右三个20dp图标、48dp点击区：进度/资料/会话设置；选择是按conversationId+executionHostBinding重建的单枚举，点击同项关闭、异项替换，一个ModalBottomSheet。原新会话入口保留于历史列表；正文不因面板打开return/重挂载。设置复用原保存/授权配置能力，资料是当前文件/引用的只读内容，不复刻全局设置；原上下文编辑仍在设置内详情，单sheet不叠面板。

本机进度reader仅调用`load_session_projection`和`conversation_plan/get`，分别验证conversation_id，读取goal_projection.active的objective/status/requirements及plan.todos。目标要求与计划分别计数，不混合为虚假总进度；未知status保留，失败/缺失分别呈现。请求LaunchedEffect按owner取消，owner变化立即清旧值；任务列表仅消费匹配sessionId和ownerSessionId的CoreTaskProjection。

边界：远程Host尚无已授权mobile Goal投影端点，显示权威进度不可用；不从本机旧checkpoint冒充远程Goal。现有匹配owner任务状态仍显示。本轮不扩移动认证协议；Room不得合并多成员Goal，当前三入口范围为明确单会话Chat页面。

本轮验证：最终APK与instrumentation APK构建成功54s（`/tmp/nola-android-inspector-final-build-r2.log`）；JVM reader/选择语义4/4通过，涵盖真实字段、两种owner不匹配拒绝、空值/未知状态不冒充完成。隔离模拟器Compose 3/3通过3.556s（`/tmp/nola-android-inspector-instrumentation.log`）：48dp点击区、同项关闭/异项替换/选中语义、真实requirements状态及1/2计数、全局文档不得混入当前资料。首轮仅测试fixture构造使用位置参数错误，已改为实际类型的命名参数，保留失败日志。

资料最终只从当前conversation消息读取canonical source/citation/attachment及归档引用；不显示全局uploadedDocuments。以上是实际组件交互与reader解码回归，不冒充完整Chat页面/实时Goal生成端到端验收；本轮未发送模型请求、未访问正式库。远程Goal端点和真实运行更新仍属未验证边界。
