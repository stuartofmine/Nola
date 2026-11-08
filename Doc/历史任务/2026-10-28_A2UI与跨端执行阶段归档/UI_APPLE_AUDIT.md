> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

> **单版本开发约束：Nola 仅保留当前架构、接口、协议和数据格式，不新增旧版本兼容层、双读/双写或历史迁移；不自动删除、重建或改写用户数据。**

# Apple UI 审计与实施

## 2026-10-19 会话检查面板（三入口）

局部设计见 `Doc/UI和交互设计/APPLE_CONVERSATION_INSPECTOR.md`，共用总契约 `CONVERSATION_INSPECTOR.md`。Mac 标题栏末端改为进度/checklist、资料/folder、设置/sliders，初始收起；正文容器保持同一结构。进度读取当前 Core Goal requirements/work、Task 及 conversation_plan.todos，不解析自然语言回复制造待办、不估算百分比。superseded 工作条目排除，reported_done 保留未验收语义。只在面板挂载时每 3 秒只读刷新；错误停止并给重试，取消与 owner 检查拒绝迟到提交。

资料承接原生附件 Quick Look、浏览器上下文与会话数据源；设置保留记忆/检索、模型、上下文用量和知识配置。iOS 增加对应进度、资料触摸入口，沿用会话设置入口；原生 sheet 不卸载正文。iOS 新资料页当前展示上传上下文文档；消息内附件和引用仍在原消息入口，不能将本轮写成所有来源聚合完成。

验证记录：

- Mac r1 `ConversationInspector-r1.xcresult` / `/tmp/nola-conversation-inspector-macos-r1.log`，11/11 通过，包含真实 Inspector 交互和 DTO 保真回归、12 张宽窄浅深截图。实际看图确认宽 1440 的进度、设置、资料正常；980 夹具未展开覆盖层，所以 r1 不计窄面板通过。已最小修夹具强制显示路径，追加真实三按钮 AX press 开启/切换/关闭测试，等待集中复验。
- Mac 后续收口：r2 **2/2**（真实三按钮 press + 12 图），r3 **2/2**（显式关闭回调 + 图），r4 **4/4**（三个真实控件交互 + 图）。r4 实际看图揭示窄 overlay 仍有独立构造未接 header 参数，不能用孤立控件通过替代工作区接线。删除重复构造，统一复用 `inspectorContent` 后，r5 `ConversationInspector-r5.xcresult` / `/tmp/nola-conversation-inspector-macos-r5.log` **2/2 通过，exit 0**。最终已实际查看真实工作区 `/tmp/nola-inspector-progress-980-light.png`、`/tmp/nola-inspector-settings-980-dark.png`、`/tmp/nola-inspector-resources-980-dark.png`：三图都有标题、三入口、当前选中项和显式关闭 X，正文仍在背后，未见横向裁切。r5 截图包含 980/1440 × 浅/深 × 三页共 12 张，不宣称每张逐一审查。r4 的真实 AX press 已覆盖资源→设置→进度→再次点进度关闭，以及显式 X 回调；最终工作区截图验证实际接线与可见性，不虚称工作区整体点击的端到端 UI 驱动。
- iOS r1 编译失败（UUID? 未转 Core String owner），没有执行测试；已使用 canonical lowercased UUID 修正。r2 `ConversationInspector-r2.xcresult` 实际 **4/4 通过、0 skip**，日志 `/tmp/nola-conversation-inspector-ios-r2.log`。导出 `/tmp/nola-conversation-inspector-ios-images/manifest.json` 共 8 图；已查看窄浅进度、窄深资料、宽深进度、宽浅资料四图，无裁切。范围为新原生页空态和上传文档 fixture，不含有 Goal 实景或整页顶部三个入口截图。
- 所有数据源为隔离测试库/fixture。没有实际模型、正式会话库、实体设备或工具执行。本轮 DTO 测试不等于 Host 传输恢复验收；owner 迟到保护目前为代码审读，未新增传输故障注入。

本轮定向命令（Mac 均在 `scripts/lib/xcode_test_device_guard.sh` 的 begin/install_traps 保护下运行）：

```sh
xcodebuild test -project Nola.xcodeproj -scheme NolaMac -configuration Test -destination 'platform=macOS,arch=arm64' -destination-timeout 10 -derivedDataPath .build/apple-room-validation-macos -clonedSourcePackagesDirPath .build/real-mac-xctest-derived-data/SourcePackages -disableAutomaticPackageResolution -resultBundlePath .build/apple-room-validation-macos/ConversationInspector-r1.xcresult ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO -only-testing:NolaMacTests/MacInspectorProgressTests -only-testing:NolaMacTests/MacInspectorInteractionTests -only-testing:NolaMacTests/MacVisualSnapshotTests/testThreeConversationInspectorPanesAtWideAndCompactWidths

INCLUDE_UI_TESTS=0 DESTINATION='platform=iOS Simulator,id=BAC347A0-62AF-414D-86B1-4F7B41F39336' DERIVED_DATA=/Users/gaojc/Desktop/Nola/.build/apple-room-validation-ios RESULT_BUNDLE=/Users/gaojc/Desktop/Nola/.build/apple-room-validation-ios/ConversationInspector-r2.xcresult PARALLEL_TEST_WORKERS=1 XCODE_OPTS='-configuration Test -collect-test-diagnostics never -clonedSourcePackagesDirPath /Users/gaojc/Desktop/Nola/.build/real-mac-xctest-derived-data/SourcePackages -disableAutomaticPackageResolution' ./scripts/run_client_tests.sh -only-testing:NolaTests/IOSConversationInspectorTests
```

Mac r5 与上面命令的公共参数相同，仅 result bundle 改 `ConversationInspector-r5.xcresult`，filters 为 `MacInspectorInteractionTests/testOverlayHeaderSwitchesSamePanelAndRepeatedSelectionCloses` 与 `MacVisualSnapshotTests/testThreeConversationInspectorPanesAtWideAndCompactWidths`。最终 `git diff --check` 清洁，代码冻结。

## Summary

2026-10-19，Needs work。范围为 SwiftUI/UIKit iOS 与 SwiftUI/AppKit macOS；沿 `DESIGN_SYSTEM.md` 水墨青山，阅读与身份优先，原生导航不重造。当前依据源码及此前隔离 Room 截图，不把静态审计当作全窗口/VoiceOver验收。生产数据、模型和授权协议不在本轮修改范围。

## 有证据的缺陷与实施计划

|级别|文件/证据|修复|
|---|---|---|
|Critical|`Nola/UI/DesignSystem.swift` 的 `NolaPressableStyle` 在 Reduce Motion 下仍 scaleEffect；频繁按钮整体透明度降低文字对比|共享按压反馈改局部背景，不缩放文字；保持系统减少动效|
|Critical|同文件 `NolaPageHeader` 始终横排图标/标题/动作；辅助字号挤压标题|辅助字号纵向分层，标题完整换行，动作单独一行|
|High|`ChatConversationsSheet.swift` 旧 sheet 行使用 onTapGesture，非 Button；plus 无读屏名|原生Button行、最小44pt、动作名称，不改选择owner|
|High|`SettingsView.swift` 共享链接/开关说明限制2–3行，辅助字号丢说明|大字号不截断，保留原生Toggle/Picker|
|High|`IOSModelConnectionsView.swift` endpoint使用普通键盘/自动更正，秘密输入未禁止自动大小写|URL键盘、关闭自动更正和自动大写；不改账户事务|
|High|`MacPageChrome.swift` 聊天列1200/正文820，偏离880/720阅读基线；共享panel给所有管理块阴影|统一阅读列与输入轴，移除内容层阴影|
|Medium|`MacSidebarComponents.swift` 会话hover缩放、pressed动画忽略Reduce Motion；标题单行无tooltip|静态局部hover、尊重Reduce Motion、标题tooltip与40pt行密度|
|Medium|`MacCodeReferencePreview.swift` 文件名单行且header固定52pt|允许两行、完整名tooltip、最小高度而非固定裁切|
|High|`MacHostPermissionView.swift` 执行位置local蓝/remote绿暗示远端成功，授权卡边界orange×0.24且内容阴影|位置用中性文字，保留警告图标；surface+controlOutline明确边界，16pt内边距，移除内容阴影；不改任何批准/拒绝回调|

依据：`accessibility.md › Vision`“Support larger text sizes”；`layout.md › Adaptability`“Be prepared for text-size changes”；`motion.md › Providing feedback`“In apps, generally avoid adding motion to UI interactions that occur frequently”；`buttons.md › Best practices`“Always include a press state for a custom button”。本轮已完整读取基础、双平台、motion/buttons/settings/sidebars/entering-data 共11份参考。

## 保留与边界

保留原生Settings导航、现有模型账户owner提示与取消、Chat/Room恢复及授权回执、工具默认折叠、文件QuickLook与访问边界。iOS文件预览使用QLPreviewController，无证据不重造；Mac授权按钮已有语义和固定错误处理，不为了颜色改执行。授权卡滚动可见既有证据不等于点击/读屏验收。

## 验证

统一修改后顺序运行隔离Test配置与组件截图；不覆盖签名Debug或访问用户库。

macOS r2：`AppleUIAudit-r2.xcresult`（`.build/apple-room-validation-macos/`），49/49通过、无跳过，日志`/tmp/nola-apple-ui-audit-macos-r2.log`。执行MacChatBubbleLayoutTests、MacRoomAndConnectionVisualTests、MacCodeReferenceVisualEndToEndTests、MacMarkdownParserTests和通知设置/生产workspace两项截图测试。实际查看生产workspace、代码预览、浅深授权卡、通知设置、Room长标题浅深/空态深色/共享回复深色、账户页浅深及错误/加载状态。授权卡拒绝与允许按钮在图内，不代表点击或读屏通过。

Mac截图限制：账户页截图当前只覆盖上半部导入/授权说明，下面的实际连接列表未滚动入图，因此不能以fixture名称声称长连接名/目录行已视觉通过。代码预览使用普通文件名，超长文件名换行仅源码覆盖。通知设置下方内容和其他设置pane、独立账户sheet没有本轮截图。截图导出目录`/tmp/nola-apple-ui-audit-macos-images/`，通知设置另存`/tmp/nola-notification-settings.png`。

首轮Mac未进入XCTest：Runtime无terminal-ui构建发现CLI取消monitor错误引用受feature gate保护的interaction；已交所属代理修复三处feature边界，不在Mac启用终端UI。首轮iOS也未执行测试：新增fixture试图写只读Reduce Motion环境及可选附件名，已修为不伪造系统Reduce Motion状态、明确解包附件名。Reduce Motion本轮静态核对共享样式，没有实际系统开关录屏证据。

实现增量：设置辅助字号的开关行改纵向，说明不截断；常规导航图标统一交互accent，危险动作保留危险语义。共享Disclosure触摸目标至少44pt且展开仅淡入不缩放。iOS组件fixture追加AX5滚动到底的截图，避免只看到页首便宣称下方设置已验收。

iOS r2四项通过后实际查看截图，发现320pt默认通知说明仍被两行截断，AX5链接保留左右图标使主标题压成两字列。随后取消说明行数上限，并让共享标题/设置链接/空态在辅助字号采用纵向布局，把宽度留给文本。该修复需要最终r3复验，r2不能充当最终证据。

iOS最终r3：`.build/apple-room-validation-ios/AppleUIAudit-r3.xcresult`，4/4通过、无跳过；日志`/tmp/nola-apple-ui-audit-ios-r3.log`。IOSDesignSystemVisualTests使用真实共享组件，320/834pt × 默认/AX5 × 浅深色8张，加AX5滚动底部4张，12张已逐一实际查看：默认通知说明完整；AX5标题/说明使用可用宽度，底部Toggle及空态可见，无横向溢出。这里是管理组件组合fixture，不是完整模型或设置导航页面。另复跑IOSRoomFullPageTests与两个真实UIKit输入命令用例；导出12张Room图，本轮复看390浅色候选和390深色授权操作图，拒绝可见，未点击执行授权。全部24张附件在`/tmp/nola-apple-ui-audit-ios-r3-images/manifest.json`。未调用模型、未触实体设备、未操作用户库；未验证VoiceOver朗读顺序、真实系统Reduce Motion开关或所有平台窗口。

审计覆盖映射：导航与会话列表（原生按钮/侧栏反馈）、普通Chat及Room（共享阅读宽度与输入按压/既有过程折叠）、模型账户（输入键盘和共享panel）、设置（共享行大字布局）、授权（明确边界、身份不冒充成功）、文件预览（Mac文件名和iOS原生QuickLook保留）。本轮不会因此宣称每个账户编辑sheet、每个设置pane、所有文件类型或每种窗口尺寸均已截图验收。

## Room拒绝操作补验

2026-10-19：既有完整页fixture仅设置pending请求并截取拒绝按钮，owner专项仅直接调用处理闭包，缺少真实控件到处理器的接线证据。首轮尝试从普通unit宿主遍历SwiftUI无障碍树，实际无法取得按钮对象，16/17通过、完整页失败，未发生点击，不能归为产品缺陷。`RoomDenyInteraction.xcresult`/`/tmp/nola-ios-room-deny-interaction.log`保留失败；其独属simctl诊断PID76382在等待子进程，按授权仅TERM诊断，原xcodebuild返回65，不删证据。

改为DEBUG且显式`-ui-testing-room-permission`入口下的隔离完整Room fixture，XCUITest查找`chat.permission.deny`并真实`tap()`，不替换生产按钮或直接调用闭包。隔离Host permission session记录真实序列化payload，并暂停传输确认；切换另一Room后释放原确认，检查原owner清除、另一owner同requestID不同nonce的请求不被消费、没有新增Room submission。对应`buttons.md › Best practices`“Make buttons easy for people to use”，不新增样式或改变批准规则。

本轮只模拟权限传输，未运行Host/Core真实工具；拒绝回调和无新增提交不等价于远端工具执行端到端证明。实体设备手指触摸、VoiceOver朗读顺序仍非本测试范围。

专用XCUITest首轮未编译通过（fixture的StateObject抛错autoclosure）；r2实际进入页面但没有卡片，截图`/tmp/nola-room-deny-ui-r2-live.png`、结果`RoomDenyUITap-r2.xcresult`保留。根因为fixture eager初始化重复替换container manager，旧StateObject持有另一execution；已改StateObject延迟工厂、复用原隔离manager，未更改产品授权处理器。r3为本轮最后一次集中运行，不以失败尝试计通过。

r3终态exit65，1项失败、无runtime warning，总89秒。真实AX层级证明权限卡已出现，但`IOSAgentRoomsView.swift`直接给复合卡片附`room.permission.<participant>`导致子级Allow/Reject/参数/输入全部继承同一identifier，覆盖原`chat.permission.deny`。证据`/tmp/nola-room-deny-ui-r3-images/D751F540-0400-4C42-8678-96E81711A695.txt`。最小修复是在卡片identifier前显式`.accessibilityElement(children: .contain)`，保留owner分组且不吞子级动作标识；尚待单次复验，不声称点击成功。未改变授权语义或执行协议。

修后r4：**真实XCUITest 1/1通过，0跳过、0runtime warning**，session13716 exit0，用例21.149秒，测试会话约72秒。`.build/apple-room-validation-ios/RoomDenyUITap-r4.xcresult`；日志`/tmp/nola-ios-room-deny-ui-tap-r4.log`。沿guard运行器、Test配置、原隔离DerivedData、单worker，命令入口为`INCLUDE_UI_TESTS=1 ... ./scripts/run_client_tests.sh -only-testing:NolaUITests/IOSRoomPermissionInteractionTests`，额外`-collect-test-diagnostics never`只关闭失败时冗长sysdiagnose，不跳过断言或附件。

证据覆盖：精确查询`chat.permission.deny`→实际向上滑动权限卡→`isHittable`→真实`tap()`→收到nonce-a/request_id/authorization-deny精确payload→保持确认未返回→点击Switch to B→点击Acknowledge A→A请求及owner清空、B同ID的nonce-b和owner保留、B传输次数0、批准次数0、两个Room均无submission。最终状态`deny-confirmed-owner-isolated-no-submission`通过内部事实计算后显示，非按钮直接写成功字符串。

前后两张图已导出并实际查看：`/tmp/nola-room-deny-ui-r4-images/16DE2076-6AFA-4959-89D9-D27AE3394FBD.png`（点击前Reject完整可见）；`D8CF5E8A-9769-4FF3-8BC4-CD4722F9EC01.png`（迟到A确认后B卡片仍在，最终验证状态）。本次点击只覆盖iPhone 18 Pro Max模拟器440pt浅色；此前390/834浅深截图不因此升级为四种环境真实点击。真实模型、网络Host和工具均未调用。

### 2026-10-25 iOS 配对焦点与真实远程任务收尾

- `WebHostConfigurationView` 的 URL 与令牌不再共用 Bool 焦点；模型参数、会话 context/output 限额也改为各字段独立焦点。Host 令牌使用明确的配对码 content type，避免 `nil` 仍触发 UIKit 登录表单推断和系统“保存密码”面板遮挡导航。完整 UI 用例现在明确拒绝该面板，不靠点击“以后”掩盖问题。
- 长令牌、长任务由 XCTest 通过系统 Paste 菜单输入，不直接写应用状态，也不再将令牌作为 `typeText` 参数写入日志。短文本仍有真实键盘焦点回归。
- 旧专用模拟器的键盘合成和 AX 查询反复出现约 150 秒超时。活进程采样 `/tmp/nola-ios-token-timeout-sample.txt` 显示主线程正常等待 CFRunLoop，不支持“应用主线程死锁”的结论。未删除旧模拟器或数据。新隔离 iOS 27 iPhone 17 `28F93564-C782-466C-9684-70C2669BED20` 的短键盘 URL→token→URL→token 回归 **1/1 通过，25.9 秒**：`/tmp/nola-ios-fresh-focus.xcresult`。
- `/tmp/nola-ios-pair-code-booted.xcresult` 实际完成配对、无密码面板、返回 Chat、冷启动配对恢复、选择 Host、系统粘贴任务、发送、返回会话列表再进入，以及两次真实 Allow 点击。**完整用例仍失败**：双子任务的总业务耗时超过原 300 秒期限；没有扩大期限。后续四轮对话与手动压缩未执行，不能算通过。
- 随后对原会话 `7526902D-8C5A-41E3-90DE-B7ACC2C09642` 冷启动恢复，未重新提交任务。`/tmp/nola-ios-fresh-cold-recovery.xcresult` **1/1 通过，30.8 秒**。Host 原父 Run 及两个子 Run 全部 sealed；`task-536e3f810b57` 返回 `CHILD_ONE=323`，`task-8c876a7c7a63` 返回 `CHILD_TWO=667`。最终截图实际查看：`/tmp/nola-ios-fresh-cold-evidence/AEE287D3-D2DF-461F-B272-BCB009C6CD46.png`。
- 并发验证 **不通过**：第二个 write-capable 子任务在第一个活跃时受项目并发上限拒绝，后续顺序完成。最终模型也明确说明此限制；两次真实执行和冷恢复成功不等价于并发执行通过。原日志、失败结果和隔离 Store 保留。

#### 原会话第二次发送与只读并发（继续验证）

- 真实 UI 在原会话第二次发送发现 `dispatch_not_admitted`，Host 未创建新 Run。根因是远程请求遗漏 Host 的 `expected_configuration_revision` 和 `expected_parent_message_id`；设备镜像配置 revision 为 2，Host 为 1，不能拿设备 revision 代替。现由 Core `ObserveSubmissionFrontier` 原子读取权威配置与可见末消息、校验既有 admitted owner，HTTP 只转发。iOS 用此前 canonical terminal 的 Run/assistant witness 查询，并验证返回 parent 与发送前显示的 parent 一致，再一次冻结入 durable payload；重放不重新观察、不偷偷 rebase。
- iOS 编译及 8 项定向边界测试通过：`/tmp/nola-ios-frontier-unit.xcresult`。原会话真实 UI 第二次发送已被新 Host 接纳，Run `669bec96-3e92-4270-a1f2-c3fea8d3106b` 创建成功。
- 两个模型实际选择 `permission_mode=read_only` 的子任务 `task-dbba84adf796`、`task-d62cf37a3862` 均 sealed，返回 `READONLY_ONE=1147`、`READONLY_TWO=1763`。Core authority 均归属于该原会话和同一 parent Run，metadata `requested_read_only=true`、`can_mutate=false`；执行区间重叠 **24.296 秒**。证据 `/tmp/nola-ios-readonly-store-evidence.json`。
- 该只读并发完整 UI 用例仍 **失败**：父 Run 最终总结的 `model_stream` 长时间 pending，原 240 秒回复等待期未出现唯一 marker；未扩大期限。`/tmp/nola-ios-frontier-readonly-owned.xcresult`，262.9 秒。子任务并发事实不等价于父总结及 UI 全流程通过。四轮对话和手动压缩还没有执行。
- 后续父 Run 自然完成 `answered`（`completed_at_ms=1792925800367`），发生在新增 Stop 用例编译期间。因此 `/tmp/nola-ios-frontier-stop.xcresult` 未找到运行中停止按钮，**不计取消覆盖**，没有伪造停止成功。
- 原会话后续四轮唯一 marker 的真实键盘发送及回复均完成，证明多次 frontier 冻结/归属持续可用。`/tmp/nola-ios-frontier-compact.xcresult` 在点击 Run compact 后失败：结果区位于 Form 屏外，测试未滚动，且 Host `auxiliary_operations` 仍为 0。进一步只读核查：Host 会话设置只有 `project_root`，每轮模型通过 `model_intent` 冻结；现有 paired compaction HTTP 仍硬要求保存的 model/modelProfileId，可能在执行前拒绝。此项未完成，不宣称压缩/压缩后回忆通过。
- 将 overview 的维护反馈放在 compact 按钮所在 Section，其他面板仍保留自己的反馈；依据 apple-design `feedback.md › Best practices` 关于状态反馈靠近相关操作的原则。测试拆出仅压缩已有历史的用例，先实际 Reload Host state 再执行并滚动读取结果，避免重复发送已完成的四轮。
- `/tmp/nola-ios-frontier-compact-visible.xcresult` 冷重开后进一步暴露本地维护入口仍依赖短寿命 `remoteSubmissionIntent`/内存 receipt：UI 明确显示 `Host context maintenance requires a confirmed conversation admission`，未到达 Host。现将原 frontier 的已提交 terminal owner 查询抽为 `confirmedTerminalOwner`，两处复用；canonical 消息只提供 Core 查询 key，根 Run/assistant 必须由 Core 已提交 terminal 返回。没有用当前新 intent 或 UI 文本推断 owner。待统一新 Core 构建实测。
- Apple 冷恢复授权改用共享 Core `validate_run_recovery_authority`，保留 owner/digest 严格回执校验；8 项 ExactInvocation 测试通过。另一个现有 CommandEffect fixture 写死旧协议 52，当前为 53，导致 3 断言失败；fixture 改用当前版本常量和 Application create DTO（不是 Store DTO 的 revision 字段），没有添加旧协议兼容。
- 新 Worker 身份采用 Core 编码的 `process` / `unobservable` 能力。Apple 只读取 OS boot-session UUID 和进程 starttime；拒绝从墙钟或 uptime 合成身份。沙盒读取失败使用 typed unobservable，正常实时执行仍可用，不能据此证明冷进程死亡。未知/EPERM 不能当 gone，PID 复用判断交给 Core。真机能力尚未验证，不以模拟器代替真机结论。

### 2026-10-25：原会话冷开手动压缩与真实回忆

- `/tmp/nola-ios-compact-cold-owner.xcresult` 实际点击原会话的 Reload context state / Run compact，约 76 秒显示 `Host context compacted`，保留 receipt 截图。此前同会话四轮真实问答已完成。
- 随后发送不包含期望 task ID/结果的唯一回忆问题；模型选择 `conversation_recall`，在 Device Hub 的模拟器界面展开参数并实际点击单次允许（隔离 Host 的测试历史）。Host Run `7b2d3464-175c-45f3-b04c-c99f94251b0c` 已 sealed；新 assistant 最终回复包含原始 `task-536e3f810b57 / CHILD_ONE=323` 与 `task-8c876a7c7a63 / CHILD_TWO=667`，也准确区分了后续只读任务。
- 本次 XCTest **仍记为失败**：唯一失败是 contains-marker 的 bubble 查询匹配 2 项，四个新回复事实断言均通过。已收紧为回复以唯一 marker 开头，避免工具历史预览中的引用命中；尚未把该修订声称为重新测试通过。
- Xcode 27 的模拟器可通过 `com.apple.dt.Devices`（Device Hub）访问 iOS AX 与真实点击；并非 Simulator.app。上述审批确实由界面完成，无直接写 Store/伪造结果。

### 2026-10-25：远程首增量后真实停止

- `/tmp/nola-ios-remote-stream-stop.xcresult` **通过**：原配对会话真实发送 150 段长输出请求，观察新唯一 marker 的首文本，确认按钮仍为 Stop generation 后真实点击，再验证 UI 退出运行状态。
- Host Run `51e73345-2c01-431e-a3cc-61fd419de197` 已 sealed；精确 terminal seal 的 result 为 cancelled，unknown_effect_refs 为空。证据 `/tmp/nola-ios-remote-stop-evidence.json`。曾观察到 result_decided 后短暂 running，随后正常封存，不记录为持久卡死。
- 另静态审查发现 Apple 本地冷恢复在等待旧 lease 时尚未 runActivated，会使用户 Stop 被本地 guard 提前拒绝。已改为在 Core 冻结身份和 active_run owner/digest 校验后保存取消身份，等待/claim 不授予任何提前执行权限；Core 仍校验取消命令。新增等待中取消身份测试，当前正在构建验证；远程通过不能替代本地冷取消实测。

- 本地冷恢复取消身份单测 `/tmp/nola-ios-cold-cancel-identity-unit.xcresult` 本轮**未执行**：统一 Rust 构建期间共享 `interrupted_effect_delivery_candidates` 接口并发由 3 参数变为 4 参数，编译器 E0061，缓存层同时报告输入文件编译期间改变。保留失败日志，待 Core 稳定后统一重编，不跳过指纹校验。

- 同批检查 Apple 本地恢复发现 schedule 的 Resume 分支此前直接 claim，遗漏 Core 的 ResumeHostEffect 重绑。现统一调用 `prepare_run_effect_recovery`；有重绑时刷新精确 invocation、重新执行 Core authority 校验、读取新 committed requests，随后以新 checkpoint witness 读取 control/tasks。不会直接重放旧请求、推断成功或放宽未知 effect。Core 稳定后的统一构建正在进行。

- 下一轮统一 Rust 构建已完成，但 Xcode 在 C 编译处报告缺失 `NolaCore.o.scan`，Swift 新测试尚未执行；保留 `/tmp/nola-ios-cold-cancel-identity-unit-final.log`，不声称测试通过。
- 同批对齐 Android 真实恢复发现：Apple 两个 recoveryPreparation 闭包不再拒绝 Core 合法推进的三个 checkpoint witness 字段，其余冻结 invocation 字段保持严格相等，并执行 Core authority 校验。普通模型和 compaction 都消费 Core `attempt_started`：恢复已开始的物理 attempt 使用新 ID，尚未开始的 prepared attempt 继续原 ID。上述补丁仍待统一构建/验证，已通过的远程业务记录不替代本地恢复验证。

### 2026-10-26：Apple 收尾构建与边界回归

- `/tmp/nola-ios-cold-cancel-identity-unit-verified.xcresult` **TEST SUCCEEDED，16/16**：CommandEffect 5 项、旧 lease 等待阶段 exact 取消身份与 Prepare-before-schedule 1 项、ExactInvocation 10 项（含 checkpoint 合法推进、冻结字段篡改拒绝、remote terminal owner）。本轮正常重试已重新生成缺失 C 依赖扫描文件，无删除用户数据或指纹绕过。
- 完整 Apple app/UI test target 编译通过，包含普通模型/compaction 的 `attempt_started` 处理、两处 ChatManager frozen witness 校验修复。
- 边界：以上单测与编译不是本地冷进程终止后的端到端成功证明；Apple 本地冷恢复/死亡证明新补丁尚未实际完成一次 UI 复测。此前远程首流停止有 UI XCTest 与精确 cancelled seal；冷开压缩/原事实回忆有真实 UI receipt 与新最终回答，原 XCTest 的重复 marker 定位失败保留，不改写为全部通过。

### 2026-10-26：本地冷恢复入口与共享 Unknown inspection

- 实际本地模型导入/发送已执行；`/tmp/nola-ios-local-cold-real.xcresult` 中能力边界 5 项通过，但长请求首文本等待失败。原 Run `daf0b3cc-e6c0-4a60-baf0-19b25f2f72dd` 保留：01:44:24 主机发生 Clamshell Sleep，154 秒后唤醒，模型 delivery lease 已超期。该样本不能用作模型首字延迟测量，也没有覆盖到计划中的进程退出/Stop。
- 随后的原会话真实冷开仍没有 Stop。发现两个入口问题：ChatManager 仅恢复 waitingForPermission；恢复入口只要 Unknown inspection 非空便退出，而共享 Store 对普通 pending 请求返回 `targets: []` 的非空 inspection。前者已改为恢复所有受 Core 精确身份约束的本地 active Run；后者已在共享 Store 修为没有 OutcomeUnknown target 则返回 None，不在 Apple 自行过滤或猜测恢复策略。
- `unknown_effect_user_decisions_commit_atomically_and_replay_without_reexecution` SQLite/Application 回归通过：真实 ExecutionStarted 的 pending 请求不构成 Unknown；真实 orphan recovery 产生的 Unknown 仍保持精确 fencing、篡改拒绝与原子决策。混合多工具并发 Unknown 场景尚未新增成功覆盖，不将无效 outbox fixture 当作通过证据。
- `/tmp/nola-ios-local-cold-stop-fixed.xcresult` 仍是失败记录（当时产物未含共享 inspector 修复），不能写为本地冷停止通过。新 Goal 当前格式加入必填 control_generation 后，旧失败测试库保留，不自动转换；使用新隔离 namespace `real-local-cold-inspector-20261026` 继续实际导入、冷 Stop、同会话再发测试。

- 新隔离真实运行 `f334bfa3-9b9b-4a50-a8fe-6796a2fef43a` 已收到首文本，XCTest 在 t129s 实际 terminate，t130s 冷 launch、t141s 重入原会话。仍无 Stop 的第二原因已定位：当前 Core exact invocation 增加必填 `remote_tool_plan`，Apple strict keys 未同步。已接当前必填 object/null 字段并纳入冻结内容比较，不保留旧字段集回退。
- `/tmp/nola-ios-local-cold-exact-plan-retest.xcresult` 中 ExactInvocation **11/11 通过**；真实 UI 已在 t15.55s 点击 Stop，但日志 `Cancellation remains unresolved: Nola Core v2 returned an invalid transition`，Core 仍 running，后续 842 未产生新 Run。此轮明确失败，按钮变化不是完成证明。
- 继续定位到等待旧 lease 尚未进入 drive 时，Coordinator 未设置 `activeTurnId`；取消控制 envelope 的真实 turn_id 与本地 nil 比较被拒绝。现于共享 Core `validateRecoveryAuthority` 成功后保存冻结 runID，已有 owner 必须一致。补丁待下一真实同会话取消验证，不声称完成。

- `/tmp/nola-ios-local-cold-turn-owner.xcresult`：ExactInvocation 11 项再次通过。真实冷开后，原先 UI 发出的持久化 Stop 被自动恢复并最终封存；读取精确 terminal seal `/tmp/nola-ios-local-cold-cancel-seal.json`，原 `f334...` 的 `result.value.kind=cancelled`、`unknown_effect_refs=[]`，completed_at 对应 10:59:12。该 XCTest 随后仍等 Stop 按钮而失败（取消已经完成），故不声称整个测试通过。
- 已新增明确针对“已确认冷取消后继续发送”的真实 UI 用例，复用同会话发送 `IOS_LOCAL_AFTER_COLD_STOP=842`，等待实际回答和终态。不会用状态按钮代替模型回答或伪造 Store 结果。
- 同轮观察到旧 trace sequence 恢复没有初始化 cursor：tool.progress seq9 / run.cancelled seq10 被本地 expected1 拒绝。最终 seal 与状态正确；此问题单独保留，不能用它否定持久化完成，也不能声称 trace 恢复全部正确。

### 2026-10-26：本地持久取消恢复后真实继续发送通过

- `/tmp/nola-ios-local-cold-followup.xcresult` **TEST SUCCEEDED**：11 项 ExactInvocation + 1 项真实 UI，UI 用例 36.953 秒。冷打开原隔离会话，实际输入并点击发送，观察到真实 `IOS_LOCAL_AFTER_COLD_STOP=842`，UI 回到终态。
- 新 Run `256c655e-55f0-49da-ae8c-19aaa1eb11ed` 精确终态为 **answered**，证据 `/tmp/nola-ios-local-cold-followup-seal.json`；此前原 Run `f334bfa3-9b9b-4a50-a8fe-6796a2fef43a` 精确终态为 **cancelled** 且 unknown_effect_refs 为空。两者属于同一会话 `4B55BC90-D683-4033-81D8-AB5CCF944CA0`，没有清库或伪造回答。
- 验证范围：实际 Stop 首次因客户端缺失恢复 turn owner 被拒绝，修复后下一次冷开恢复该持久取消并完成，再次真实发送成功；本轮没有重新制造 fresh lease 窗口后再点击一次 Stop。先前失败 xcresult 保留。旧失败发送所保留的草稿也被测试输入追加，新消息长度 88；这不影响实际请求/最终回答证明，但不作为单次文本替换行为验证。
- 后续独立问题：trace cursor 恢复 seq9/10 对 expected1；真正多工具 mixed Unknown 的新增回归尚缺。本地取消测试不覆盖此次并行修改的远程 task guidance HTTP 新契约。

### 2026-10-26：冷恢复诊断 Trace 连续性

- 真实取消库的 trace_journal 中 seq1…10 完整，缺口发生在 Apple 冷恢复观察入口：只恢复 control/tasks/requests，未加载此前诊断事件，新的 seq9/10 因 reducer expected1 被拒绝。
- 新增共享 Core `load_exact_run_trace_replay`，在 pinned read snapshot 中验证 conversation/run/assistant owner、唯一 trace 与连续事件。首分页固定 through_sequence，后续页不会混入并发新事件；每页最多 256 条，事件 JSON 身份同 SQL owner 交叉校验。各平台可复用，未在 iOS 接受任意 gap 或猜测 cursor。
- Apple 在 Core 恢复权限验证后重放该 Run 的已提交诊断 trace，再处理增量；同一恢复 Run 的重复前缀去重，新 Run 不沿用旧 cursor。正文和思考内容仍走 assistant journal，trace 不覆盖正文。
- Core 真实 SQLite 回归：冷开分页、并发追加后固定 frontier、下一增量连续、外来 owner 与 journal gap 拒绝通过（`/tmp/nola-exact-trace-replay-test.log`）。Apple 新增跨页 reducer 测试和真实 `f334...` 取消库隔离副本跨 FFI 重放测试，无模型请求。
- 首次 native 验证 `/tmp/nola-ios-recorded-trace-replay.log` 因并行 Core lib.rs 修改被编译缓存明确拒绝（input modified during compilation），测试未执行；未跳过指纹验证。等待 Core 最终稳定后正常增量验证。

- 最终 `/tmp/nola-ios-recorded-trace-replay-verified.xcresult` **TEST SUCCEEDED，13/13，0 skipped**。`testRecordedColdCancellationTraceRestoresThroughActualTerminal` 实际使用此前真实取消 Run 的隔离 SQLite backup，经当前 Core FFI 读取 seq1…10，fresh reducer 全部 applied 到 run.cancelled；重复末条为 duplicate，终态后续读为空。测试不是手写十条事件，也没有重新请求模型。
- `testRecoveryTraceReplayPinsFrontierAndFeedsNextIncrementWithoutGap` 验证跨页 through cursor 固定、恢复完成后的下一增量继续 applied，以及外来 owner 拒绝。Core 最终含 event_json 伪造身份拒绝的 SQLite 回归再次通过：`/tmp/nola-exact-trace-replay-final-test.log`。
- 本次 iOS 完整编译也纳入并行完成的远程 task guidance 当前调用者改动；这仅证明编译兼容，不能替代 guidance 真实远程任务验收。

### 2026-10-26：现有配对 Host 真实短消息与回连

- 复用隔离模拟器及原 5148 配对，不清数据。两次 XCTest 真实进入同会话、点击输入并发送，均通过：`/tmp/nola-ios-remote-short-first.xcresult`、`/tmp/nola-ios-remote-short-second.xcresult`。
- Host 新增 exact Run 分别为 `7cf76ed1-24fb-422d-8bd7-7a743ba1b533`、`e2e84fd7-a391-4f00-b704-7efe1981a1f0`，都 sealed answered；隔离 backup 通过 Core 读取 terminal seal 与最新 canonical tail，确认每次独立新增回答 `IOS_LOCAL_AFTER_COLD_STOP=842`，并非仅匹配旧 UI 文本。
- 两轮之间实际启动 Settings 将 Nola 切到后台，再回到同 PID 的 Nola；另一次 XCTest 真实冷启重新进入会话。截图 `/tmp/nola-ios-remote-background-settled.png` 与 `/tmp/nola-ios-remote-foreground-restored.png`。未注入网络断线，不宣称断网恢复已覆盖。
- 同时发现列表伪“已受理待确认”警告。日志证明失败的是只读 preview；旧隔离本地会话 `2822E16E-58E2-438E-A62E-16A953894D5B` 的 checkpoint 缺当前 `control_generation`，Core 明确格式拒绝，合法远程会话读取成功。所有 pending journal 为空。未转换或删除旧数据。
- 修复只读错误被统一 decoder 错误转为 outcomePending：复用现有 read-only command 分类保留 Core 错误，mutation transport uncertainty 保持原样。预览错误按会话行展示，不污染其他会话。新增定向回归，Swift parse 已过；新版 native 与 UI 验证尚待运行。

- 新包真实 UI `testExistingHistoryReadFailureIsScopedToItsConversation` 通过（16.238 秒，`/tmp/nola-ios-remote-short-read-error-scoped.xcresult`）：旧格式错误只属于对应会话行，实际点击合法远程会话后历史仍可读；无模型请求。附件已导出到 `/tmp/nola-ios-read-error-ui-attachments`。
- 同轮 receipt suite 旧 fixture 硬编码 service protocol 52，而当前生产为 53，导致 3 个旧测试共 6 个断言失败；新增只读失败回归通过。已改 fixture 读取当前 expectedServiceProtocol，无旧版兼容；整轮不能标绿，重新定向验证另记录。
- 修正 fixture 后，实际生产 decoder/classifier 与原样 recoverApplicationEnvelope、完整 receipt XCTest 文件组成的独立 Swift package **8/8 通过**（`/tmp/nola-ios-receipt-package-tests.log`）。覆盖读错误保真、只读 accepted 拒绝、mutation 外层错误仍 pending、原回执查询与精确重放。该验证不重编 Core，也不替代前述真实模拟器 UI。

### 2026-10-26：子任务结果的只读导航（待统一 native 验证）

- 移除 iOS 对 `result_ref` 任意 URI 的 `Link`。任务卡改用 Core TaskHandle 的 parent conversation、parent Run、task、child conversation 四元组发起导航请求；结果页不提供输入/执行控件，返回时父会话与草稿不变。
- Host 复用现有 authenticated encrypted `/chat/agent/v2/journal` 增加 `task_child_window` 只读 operation，直接映射共享 Core `load_task_child_conversation_window`；Core 校验实际任务归属，再读取已有窗口。Apple 核对返回 `execution_ownership` 完全匹配，复用现有 journal reader 水合完整文本与公开 reasoning，不解析 opaque URI、不伪造 child admission、不写镜像会话。
- `Shared/Protocol/ConversationExecutionOwnership.swift` 为 Apple 共用严格 DTO；缺字段/未知kind明确拒绝。普通远程根会话 ownership 的可信 admission 接收由共享 Core 实现，不能以缺本地 task 推断 User。
- 共享 DTO 定向测试 2/2 通过（`/tmp/nola-apple-task-ownership-tests.log`），相关 Swift parse/typecheck 通过；新增 Host exact scope mapping/reject-URI 测试及实际点击结果→返回父会话 UI 用例。尚未统一 native 编译或真实结果页点击，不能标验收通过。新 epoch 6 真验收使用新隔离 namespace，旧数据/旧验收证据保留。
- 验收边界：本轮只验证 remote 子任务结果页，不代表 iOS 本地 guidance 实时呈现已通过。parent guidance metadata 由独立 parent journal 变化发布，不能将 foreign parent hint 塞入 child consumer；本地现有观察入口是否订阅该 parent 变化仍需定向核对。
- 新 epoch 6 native 编译成功；修正两份共享历史 projection fixture 的明确 ownership 后，34 项 native receipt/ownership/conversation projection 测试全部通过（`/tmp/nola-ios-epoch6-task-result-retest.xcresult` 内单测套件）。UI 尚未进入 child：配对后原 SecureField(.oneTimeCode) 确实触发系统“保存密码？”Sheet，截图 `/tmp/nola-ios-pair-prompt-diagnostic/C7E15AC6-FB22-40F5-8D9E-AD21097891EF.png` 和命中点 app hierarchy 已证实，不是仅凭“以后”按钮推断。未点击系统按钮绕过。
- 配对成功持久化后清空临时 token、结束焦点再退出；失败保留输入。保留原 SecureField，未引入 UIKit 输入封装。实际 UI 重验无保存密码 Sheet，冷启动保留配对并完成真实 readonly 子任务：父会话 `A4FC3C19-E3FA-4E19-ACFE-CE84C10A80EA`、父 Run `55701dc2-18b4-4f17-9bcd-654902d9af81`、task `task-ed8139c9d3a4`、child `agent-4583f6e425874e6d` / Run `5920ec42-36f6-44d6-acbc-833b79a3cdcd`，结果 `IOS_CHILD_RESULT=391`，父子均 sealed。该轮结果按钮标识被父卡 accessibilityIdentifier 覆盖，随后冷启动又暴露任务卡缺失，因此尚不能声称结果页点击通过（`/tmp/nola-ios-epoch6-task-result-clear.xcresult`、`/tmp/nola-ios-existing-child-navigation.xcresult`）。
- source journal 已实际发布 sealed，Apple 却保留旧 queued：发送期任务订阅随流结束取消，journal 更新保留旧 taskProjection；冷启动更把 task/events 观察放在活 Run receive 内，sealed 会话不触发。修复采用常驻 exact journal conversation/Run/message 归属与 projection revision 门禁，paired task 观察独立于是否存在活 Run；不从父终态或答案文本推导子任务状态。新增 native reducer 回归通过，实际原任务冷启动点击继续验证中。无新模型请求、无数据清理。
- 冷入口修复后的实际 task/events 已发起，但 25 秒收尾才报 JSON 格式错误。独立加密只读取样确认 source 返回完整 sealed projection（`/tmp/nola-ios-source-task-projection.json`）；Apple task 分支使用 `bytes.lines` 丢空行 SSE 帧边界，主聊天已采用原始字节切行。现提取 `WebAgentSSELineFramer` 供两分支共用，覆盖任意网络分片、CRLF/空行、多帧、中文/emoji UTF-8 边界；该传输错误独立于前述冷观察入口遗漏。
- 最新实际闭环 **通过**：`/tmp/nola-ios-existing-child-navigation-sse.xcresult`，2 项 native 定向回归与 1 项 UI 全部通过。UI 22.721 秒：冷启动原 391 会话 → 点击 Open result → 只读子会话出现 → 实际源 child 内容包含 `IOS_CHILD_RESULT=391` → 无 composer → 原生返回父会话恢复 composer。未创建新 Run、未清理数据；截图导出 `/tmp/nola-ios-child-result-ui-evidence`。此证据确认 cold task 查询、共享 SSE framing、exact owner 卡片更新与只读导航共同工作。另终态卡按 Core state 隐藏残留 current_step“排队中”和暂态进度，不伪填100%。
- 最终源码再验 **通过**：`/tmp/nola-ios-existing-child-navigation-final.xcresult`，2 native 回归 + 1 真实 UI（24.445 秒）。结果文本断言已收紧到 Assistant 内容容器，排除任务指令回显；实际冷启动、只读子页、原生返回完整重复通过。包含终态 step 隐藏，最终截图 `/tmp/nola-ios-child-result-final-evidence`。未新增模型调用。

### iOS embedded 任务指导当前合同核查（2026-10-26）

- 已确认实现缺口：device-local 工具目录及 Host driver 没有 `spawn_agent` / delegated child 执行能力。现阶段不能把本地多 agent 指导列为“只差验收”；原生 Goal 亦仅展示投影，操作入口由跨端共同端口另行收敛。
- 删除 `DeviceAgentCoordinator.sendTaskGuidance` 的已移除 `AgentSubmission.queue_task_guidance` 调用，改为现有 Application `SubmitTaskGuidance` / `LoadTaskGuidanceReceipt`，复用共享 Apple durable guidance outbox 保留固定 message ID、时间和内容。parent Run 与 Core turn 分别传递，不互相猜测。
- Journal reader 曾将 `task_control` 展平为普通 `task`，丢弃 `supports_guidance` / `supports_cancellation`。现保留明确 TaskControl 类型与必填能力字段；普通 Task 不授予指导能力，指导输入与发送回调均要求 source 明确支持，实际提交仍由 Core 校验当前 immutable admission / child binding。
- Core 真实 SQLite 边界测试 `guidance_capability_changes_only_when_actual_child_binding_commits` 通过（1/1，`/tmp/nola-guidance-current-core-boundary.log`）。Apple 本轮 native 编译通过；43 项测试 35 过、8 个既有 fixture 因未回答当前 `encode_host_worker_identity` 失败。新增指导/TaskControl 两项与 reader 全部 12 项（含 true/false 能力保真）通过。fixture 更新当前 worker 编码合同后的集中 native 复验已 44/44 通过（31 Application 合同、12 journal reader、1 remote tool-plan clock），证据 `/tmp/nola-ios-guidance-clock-final.xcresult`。之前失败记录保留，不用当前通过覆盖当时证据。
- 未完成：本地 delegation Host；独立 parent journal 更新订阅；本地子任务取消仍使用当前有效 `AgentSubmission::RequestTaskCancellation`（protocol.rs 明确存在），按钮同步要求 Core `supports_cancellation`；不能将其误当已删除指导枚举。Mac `task_guidance` 是有效 desktop control frame，并非同一个已删 Core 枚举；其固定 intent/receipt 重试缺口单独跟踪。

### 远程 Host 工具计划时钟归属

- iOS / Mac 远程工具计划客户端不再用本设备 wall clock 重新判断 Host 签发时间与到期时间。新请求的新鲜度仍由执行 Host 在准入时以 Host 时钟严格验证；客户端只验证 Core `validate(None)` 对应的结构、owner、schema 与最长 300 秒有效区间，不自行扩展或刷新凭据。
- 删除 Apple 客户端 `nowMs` / `requireFresh` 参数及全部调用；`issued > 0`、`expires > issued`、安全整数和 TTL 上限保持。测试覆盖过去及远未来 Host 时间可被解析、倒置时间/超 TTL/越界/身份或 schema 损坏拒绝。本机 device tool consent 的时间验证未改。
- iOS Simulator native remote tool-plan 测试已通过，纳入 `/tmp/nola-ios-guidance-clock-final.xcresult` 的 44/44；Mac 当前只完成源码/解析检查，尚不计 Mac XCTest 通过。

### iOS paired 进度来源修复

- Inspector 原先无论会话执行位置都读取本机 session/plan，paired Host 的实际进度可能被显示为空。现经 ChatManager 既有 pinned provider 选择源：paired 使用现有 encrypted `chat/agent/v2/progress`，local 仍读当前 Core；source Host、conversation、plan owner 和当前 response 字段均严格核对。
- progress endpoint 直接返回 source projection，与 journal 的 `{source,result}` envelope 不同；共享 HTTP reader 用明确的 endpoint 合同选项解码，保留认证加密与 source 校验，不用响应猜测或兼容回退。remote 无 task 字段，不从本地镜像补造任务事实。
- 同 owner 刷新/重试保留已展示事实，错误以旁边的重试提示呈现；切换 owner 才清空。Inspector 不再擅自 lower-case conversation identity。
- Swift 增量编译通过；Inspector 6 项 native 测试通过。5158 真实 UI 新配对/冷启/发送计划/打开 Progress/刷新通过，源 Run `c264cdc0-474e-4d7e-aaeb-59832fd9c52b` sealed，会话 `C80775C0-D238-44DC-A850-0DF3C71102E0` 的 durable plan 含两个 pending 项，截图 `/tmp/nola-ios-paired-progress-evidence/93C919A4-544C-4259-9DFA-B230F100BB9E.png` 已目视确认。原 UI marker 断言可能匹配 sheet 下方 prompt，因此增加原会话 cold 只读复验，改为实际步骤文本且必须 isHittable；不再发模型。此轮附带 shared guidance 契约一项因 NSNumber JSON roundtrip 的 enum case 断言失败，已交共享适配作者更新，不将整个 xcresult 报全绿。

- 最终收口：`/tmp/nola-ios-progress-cold-final.xcresult` 全绿，7/7 native（Inspector 6 项 + 最终共享 guidance prelookup 合同 1 项）及 1/1 真 UI（22.0 秒）。真实冷启动原 namespace / 原已封存会话，打开 Progress 后两个实际步骤文本均 `isHittable`，连续采样跨越刷新周期仍可见且无 spinner；没有新模型请求。最终截图保存在 `/tmp/nola-ios-progress-cold-final-evidence/AA5BF25D-BA6F-4AC6-A2A6-B256F16085AB.png`，同目录 AX 树确认 Progress sheet 内两个步骤节点；可见性由本轮 isHittable 断言验证。这消除了早前 marker 可能命中背景 prompt 的假阳性风险，并复验修正后的共享 numeric/parentRun prelookup 合同。
- 构建边界：该 iOS 产物包含 2026-10-26 17:15 task.rs 更新，不含随后开始的 source/global 分库 CancelTask 授权修复；不会把此次 progress 验收扩称本地 delegation 或跨库取消通过。

### 本地 delegation 与 CancelTask 最终静态复核（待统一构建）

- 既往移动端真实父子任务证据使用 paired CLI Host；不是 iOS 或 Android embedded 本地 spawn 验收。两端本地执行器目前均未实现 `spawn_agent`，不能把远程成功写成本地支持。
- Core 已有 Task admission、租约、公平调度、`BeginTaskAssistantTurn`、父 journal 发布与指导回执，但 mobile-bindings 仅开放 Application JSON ABI；没有跨子 Run 的共享 `CommittedRuntimeDriver` runner。
- 实际 spawn/子任务进程生命周期仍位于 CLI `task_runtime_parts/execution.rs`，依赖 CLI `HostInvocationRequest`、`AgentRegistry`、OS 子进程、进程组及 Host 凭据恢复。它不能直接链接为 iOS 可运行的子进程适配。
- 最小正确接入需要抽取共享 Rust task execution coordinator：复用现有 Core admission/claim/heartbeat/terminal/guidance，注入 Host executor port；CLI 提供现有 process adapter，移动端提供 in-process effect transport。移动平台只桥接模型流、设备操作和系统生命周期，不新建 Swift/Kotlin 子任务调度循环。还需父 journal 的独立 owner-scoped wakeup 及应用暂停后的 lease/recovery，当前未实现，属于明确架构工作量。
- 本次修复可独立收敛的真实错误：iOS 正常与 cancellation-drain 两处 `cancel_task`、Android 共用 dispatch 原先合成成功（Android甚至直接生成 `state=cancelled`）。现统一先提交 ExecutionStarted，再带最新 delivery fence 调用 `ExecuteTaskCancellationForEffect`。只透传 Core 持久化的 `cancel_requested`；不声称 worker 已停止。Core 返回 Transfer 时移动端因没有可信跨 Store transport 明确 failed/unsupported。
- 新增 iOS 2 个 wrapper 合同用例、Android 3 个合同用例；仅编辑，遵守本轮全部改完后统一编译测试要求，尚未通过验证。

### 同 Run 多 assistant segment 消费复核（仅编辑）

- 初始 assistant 保留 submission receipt、Run control 与 terminal seal 身份；新 segment 通过同 Run 的追加 message admission 证明，不能替换控制 owner。
- Apple summary 旧实现只读 root head，已改读完整 sealed manifest 的全部 assistant，验证同 epoch / conversation / Run、唯一 message / ordinal、heads_count 和 root 存在，按 canonical ordinal 拼接；每次内容读取仍 pin 精确 message owner。新增多段、缺 root、重复、foreign Run、计数不符测试草稿。
- Apple 冷恢复在 admission head 与 journal heads 之间遇到新 segment 时，会重新读取 admission 并交 Core 校验精确 message，避免合法新段因竞态漏收；不会直接信任任意 same-Run ID。
- Android journal receiver / summary 与 Web terminal reader 已遍历多 head 并验证 admission；Web 已有 additionalAssistantIds / terminal 分页回归。此轮没有编译或运行，待全部修改后统一验证。

### Embedded delegation 的可执行共享边界（代码核查，尚未实现）

现有可复用合同：

- `nola-runtime-driver::CommittedRuntimeApplicationPort`：读取/claim committed requests、独立 application control claim、renew delivery、接受物理 HostResponse、接受 boundary input；事务与恢复规则留在 Application。
- `nola-runtime-driver::CommittedRuntimeDriver`：共享 effect pump、租约 heartbeat、物理响应 ACK、parked delegated effect；不能在 Swift/Kotlin 再复制一套 child Run pump。
- `nola-host-protocol::HostDriver`：物理 dispatch / dispatch_batch，每次 emit 必须先获得 Application commit ACK；`BoundaryInputSource` 对接已持久化指导/权限输入，不负责授权。
- `RuntimeTaskCommand` 的 Create、ClaimFair、Attempt、AgentAuthority、permission suspension/recovery、ReconcileBoundRunTerminal 与 `BeginTaskAssistantTurn { lease, input }`：已有任务身份、调度与 child Run 权威入口，应直接复用。

仍在 CLI 的依赖不能直接迁入移动端：`spawn_agent` 使用 AgentRegistry、CreateRuntimeTask、预算与冻结 DelegatedInvocation；后者直接序列化 CLI `HostModelRequest` / `HostRuntimeOptions` / `AgentProfile`，捕获和恢复依赖 CLI profile resolution、SIWC 和 credential vault。`spawn_agent_task_process` / `run_agent_task` 另依赖 OS 子进程、进程组、worker heartbeat、外部执行器、CLI 权限 continuation 与终态投递。把该模块直接链接 iOS 并不能运行。

最小完整实施切面应同时包含以下接线，不能以新增无人调用 trait 代替：

1. 将 child frozen execution 描述符收敛为 Core 可验证的 model/profile revision、HostInvocation/output admission、工具/权限快照与 credential recovery **引用**；平台解析凭据但不把 secret 写 Core。复用本批 Goal/Run 的 exact binding 校验，不重选默认模型。任务 admission 与 descriptor 必须同一事务归属，原始 tool call identity 保持幂等。
2. 在现有 mobile-bindings 内增加 Rust-owned worker 生命周期与有界 physical request/response mailbox，并实现现有 HostDriver；Application handle 的 Mutex 仅保护事务，不得持锁等待 Swift/Kotlin async 回调。每条 response 以 exact Run/request/attempt/delivery sequence 接受后 ACK；同一个共享 CommittedRuntimeDriver 驱动 parent 和 child，不引入第二套 Core reducer。
3. iOS `DeviceAgentCoordinator` / Android 对应类改为 mailbox transport adapter：接收 model/device-tool/permission 请求并返回物理结果，保持各自 model streaming 能力。任务调度、ClaimFair、冷恢复、guidance、取消与 terminal reconciliation 全在共享 Rust 执行层；暂停/后台仅提供平台生命周期事实，不伪造 worker 已停止。
4. 用 Core 的 parent journal head/wakeup 独立更新任务投影，不能依赖父模型仍有 delta 或 child accepted stream。任务切换/结束始终检查 exact owner；停任务必须实际回收 in-process job 并提交 typed stop witness。
5. Catalog 只在上述 worker 与所有必要 native handler 已注册后暴露本地 spawn。验证须覆盖两只读子任务真实重叠、不同写权限审批、指导产生独立 assistant segment、父/子各自停止、后台恢复及结果返回父会话；既往 paired CLI 证据不能计为这些本地用例通过。

当前结论：此切面涉及共享 worker、FFI 执行传输和冻结 child 描述符三者，不存在仅加一个 mobile task port 即可独立闭环的接线。未宣称 embedded delegation 已完成；已修复的取消假成功与本架构缺口分开验收。本节为下一实际实现边界，未新增并行状态机、未编译测试。
