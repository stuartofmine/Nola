# 多平台逐页交互检查（2026-11-09）

范围：当前本地工作树。补充此前真实模型编辑重发、长回复验收；本轮页面夹具不冒充真实模型或生产 Host。用户数据库和凭据未改动。

## 已确认并修复

- Web / Windows 共享页面：技能与 MCP 页接收缺少 `execution_backends` 的清单时抛出渲染异常。现在先完整校验列表和后端描述，拒绝不完整快照，显示错误并允许重新读取。当前 Rust Host 的备份键允许 null，校验保留此真实合同。
- 同页保存回执也经过完整校验，未确认响应不会关闭编辑器、丢弃草稿或宣称保存成功；不自动重复提交。
- Mac：设置总览的分类按钮和返回按钮复用语义主题悬停/按下反馈及文本内边距，深浅色一致；增加真实 AX 分类进入和返回检查。
- iOS：长期记忆首次加载/快照更新原来会把整页文本换行布局一起插值，实际截图出现斜向逐字排布；嵌入式页面交给原生导航转场，快照更新不插值整页几何布局。
- iOS：嵌入式设置去掉重复的正文标题，保留导航标题。
- Android：从设置进入资料原来会清除来源导航并强制返回信息页。保留调用页面，工具栏和系统返回均弹回实际来源；信息页进入时仍回信息页。原生测试实际验证设置与信息两条返回路径。
- Mac：软件更新区域显示原始 `softwareUpdate.*` 键，补齐 21 个键及动态状态在九个语言中的文案。共享 Apple 源文件加入测试资源依赖，避免覆盖检测漏掉共享组件。

动画审查继续采用 apple-design。新增参考 `motion.md › Best practices / Providing feedback`：系统导航已有转场，嵌入式内容不再另做整页几何插值；具体取消记忆快照布局插值是结合复现作出的实现判断。未新增蓝色边框或复制按钮底色。

## 本轮检查方式

| 平台 | 检查范围 | 证据与边界 |
| --- | --- | --- |
| Web | 全部 14 路由、7 个设置分类；资料子页；源与能力编辑器；焦点恢复；错误重试；320/390/1440px × 深浅色 | Playwright 真实 Chromium/生产 Svelte 组件；隔离 API 夹具，未连接用户 Host。矩阵与当前路由清单相互校验，新增路由会要求补覆盖。 |
| Mac | 常用管理页、模型本地/API、七个设置分类、记忆/健康/通知/测试页；520/680/1040pt 与深浅色；实际设置分类 AX 点击与返回 | NSHostingView 原生窗口截图及 XCTest；不把像素大小检查当成实际交互，截图另行人工复核。 |
| iOS | 当前 7 个管理目的地 ×320/390/768pt×深浅色；输入栏和右侧信息视图 | Simulator/UIHostingController；宽度测试不等于 iPad 真机验收。像素检查拒绝纯色截图，截图人工复核；未覆盖所有设置子页和系统权限真实授权。 |
| Android | MainActivity 首次引导、四个主入口、设置分类与资料子页、进入与返回、深浅色 | 独占只读 AVD 的一次性用户。结果下文单独记录；不使用用户手机。 |
| Windows | TypeScript 构建、窗口/更新单元检查；共享 Web 页面跟随 Web 修复 | 无 Windows 系统运行环境，未完成原生系统交互验收。 |
| CLI | 本轮未改终端实现 | 前轮真实长任务/编辑器测试不等于本轮全部 TUI 页面验收；图片粘贴仍不在本轮证据内。 |

## 验证过程中暴露的测试问题

1. 旧 Web 管理页脚本寻找已移除的设置标签，设置页改为分类入口后，脚本在第一项就停止。本轮新增矩阵按当前语义链接和带作用域 ID 导航。
2. 新 iOS 页面测试最初只用 PNG 字节数判断可见内容，实际全部纯黑，不能计为通过。改为连接 UIWindowScene、绘制实际控制器并检查像素范围；保留原始结果，后续以可见截图为准。
3. iOS 非并行直连尝试遇到 Simulator `Invalid connectionUUID`，恢复现有受控克隆运行方式；首次误用 Debug 的构建已中止，仅清理该次生成的 iOS Debug 输出，未动 Mac 签名 Debug 产品。
4. Android 首次尝试直接等待主导航，实际停留在首次使用引导，失败记录保留；补齐真实引导操作，不直接写“已完成引导”偏好。
5. 同时运行两个模拟器时磁盘不足，一次 Mac Runtime 构建的 objcopy 写入失败，生成截断二进制。该次构建明确失败；仅移除损坏的生成产物并重新构建，不删除用户数据或掩盖失败。后续原生任务顺序执行。

## 结果与证据

| 检查 | 结果 | 本机证据 |
| --- | --- | --- |
| Web 页面/交互 | 184 个页面状态通过，14 路由与当前路由表一致；七个设置分类及资料子页；三种宽度×深浅色。包含异常清单恢复、未确认保存保留草稿且只提交一次、弹窗开关与焦点恢复 | `/tmp/nola-page-matrix-save-final/receipt.json`、同目录截图；`/tmp/nola-page-matrix-save-final.log` |
| Web 检查/单元/构建 | Svelte 零错误零警告；611 项通过、零跳过；生产构建通过。已有大 chunk 提示保留 | `/tmp/nola-pages-web-{check,tests,build}-complete.log` |
| Mac 首轮原生交互/页面 | 15 项通过，零跳过，104 张截图；含 28 次设置分类进入和返回的组合检查。软件更新文案修复后的最后合并回归18项全部通过、零跳过（含共享文案覆盖及更新ABI测试） | `/tmp/nola-page-mac-final-verified.xcresult`、`/tmp/nola-page-mac-final-summary.json`、`/tmp/nola-native-page-matrix/` |
| iOS 最终 | 10 项通过、零跳过。7 个管理目的地×三种宽度×两色为42张；390pt 两色再加辅助功能大字号14张，总计56张管理页截图；另有输入框和右侧信息检查 | `/tmp/nola-page-ios-complete.xcresult`、`/tmp/nola-page-ios-complete-summary.json`、`/tmp/nola-page-ios-complete-images/` |
| Android 构建/JVM | Debug 与测试 APK 构建通过；689项中687通过、2跳过、0失败 | `/tmp/nola-page-android-complete-build.log`；`android/app/build/test-results/testDebugUnitTest/` |
| Android 首轮页面 | 四个引导页及两色管理/主页面，32张；进入/返回检查通过。外观切换的一张图捕获过早，不能据此声称该图为深色 | `/tmp/nola-page-android-final.log`、`/tmp/nola-page-android-images/` |
| Android 最终两色复测 | 增加等待持久化主题选中并推进主题动画；重新运行实际 MainActivity，28张当前页面截图全部通过。已配置模型页面也覆盖；外观页暗/亮画布采样分别为(17,23,21)/(243,245,244)，已人工复核 | `/tmp/nola-page-android-theme-final.log`、`/tmp/nola-page-android-final-images/receipt.json` |
| Android 真实模型 | 使用用户现有配置，gpt-6.1-terra / Responses。1次实际请求、HTTP200、94个文本增量；实际客户端流式展示、历史持久化检查通过；没有注入中断或假模型响应 | `/tmp/nola-page-android-real.log`；临时目录 `nola-android-real-ui.PKGqHi` 中 `provider.ndjson`、`persistence-audit.json`、时间线与截图 |
| CLI 输入回归 | 6项 composer +6项 terminal_input 通过。复用已有 `nola_cli-3998fb9ecdb9fb74` 测试产物，本轮未重新编译CLI，不扩大为全部终端页面验收 | `/tmp/nola-pages-cli-{composer,input}.log` |
| Windows | TypeScript 构建和7项窗口/更新单元检查通过 | `/tmp/nola-pages-windows-{build,tests}.log`；无 Windows 原生运行证据 |

Mac 正常 Debug 构建使用项目 Apple Development 签名，严格深度验签通过，非 ad hoc；路径 `/tmp/nola-ui-polish-mac-build/Build/Products/Debug/Nola.app`。测试继续使用 Products/Test，不替换上述签名身份；新应用未启动并尝试改写用户旧格式数据库。

原生模拟器按任务创建与回收；只读 Android AVD 的第二用户没有使用用户手机。为了完成本轮验证，清理 `/tmp/nola-mobile-summary-ios/Build` 这一旧测试编译目录，保留其 Logs、TestResults 和源码依赖；没有清理用户会话、凭据或用户数据库。

### 补充的测试修正

- Mac 新测试最初错误地尝试写只读 `accessibilityReduceMotion` 环境，改为测试视图事务关闭动画；生产减少动态效果策略不变。
- Mac 共享更新文案覆盖检查最初因测试包没有 `Shared/Presentation/AppleSoftwareUpdateSection.swift` 而失败；补齐打包脚本和 Xcode 输入/depfile，保留失败的 `nola-page-mac-localized.xcresult`。
- Web 未确认保存用例最初假定默认提交按钮显式包含 `type=submit`，定位失败；改用实际表单提交按钮，没有修改生产按钮来迎合测试。
- Android 最初主题截图只等待 Compose 空闲，未等待 DataStore 主题选中；修正等待条件后两色复测通过。最终页面测试不导出可编辑字段的完整无障碍树，避免把密码输入内容写入页面记录。

### 验收边界

本轮不是“所有平台所有按钮全部通过”。未完成 Windows 系统运行、物理手机、所有权限真实授权、所有 iOS 设置子页/表单提交、所有字号和语言的逐页操作。模拟/空列表/加载失败状态不能替代真实资料、模型账号登录、后台恢复和长期任务的全部组合。CLI 图片粘贴与全部权限引导也没有本轮完整证据。前轮 Mac 编辑重发和长任务真实模型结果见 `MAC_EDITED_RESEND_REAL_ACCEPTANCE_20261109.md`，与本轮页面结果区分记录。
