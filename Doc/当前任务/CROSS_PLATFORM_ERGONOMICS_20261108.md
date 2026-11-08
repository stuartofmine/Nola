# 跨平台高频交互与人机工程优化

日期：2026-11-08；本地当前工作树。范围为 iOS、macOS、Android、Web、CLI、Windows 宿主。目标是减少误触、恢复焦点与阅读位置、完善文本编辑和窗口操作；本记录不代表所有页面和设备均已验收。没有改变协议、数据格式或用户数据库，也没有增加旧接口兼容层。

## 改动与依据

| 平台 | 具体修复 | 所有者与边界 |
| --- | --- | --- |
| iOS | 用户消息复制/编辑优先完整文字，空间不足收紧复制标签，大字号可纵排；编辑按钮补边距并保留至少 44pt 命中区域；复制成功计时可取消，尊重减少动态效果 | 保留原消息 owner/revision；页面消失或消息更换取消旧反馈 |
| Mac | 连续复制重新计算 1.8 秒反馈；更换消息和销毁时清理计时与读取任务 | 保持透明无边框复制动作，不写领域状态 |
| Android | 用户消息复制改为紧凑图标，48dp 命中区域；复制/成功状态具有本地化无障碍描述，助手保留文字动作 | 分开点击区域，不改变正文或编辑行为 |
| Web | 搜索关闭恢复原入口焦点，忽略输入法组合键；时间线可键盘聚焦；滚动合并到渲染帧并随视口大小变化保留阅读/跟随意图；连续复制重新计时并清理过期反馈 | 焦点受代次和当前页面约束；ResizeObserver 只观察视口，不宣称覆盖任意图片内容尺寸变化 |
| CLI | 组合字符/表情安全编辑；按词移动和删除；普通 Home/End 当前行、Ctrl+Home/End 整篇；重复导航和字符输入；过滤系统修饰键，Ctrl+J/M 精确匹配 | 模态编辑复用编辑器；重复 Enter 不重复提交，文本编辑不扩展授权合同 |
| Windows | 原生编辑、缩放和全屏菜单；Ctrl+Q 明确退出；恢复窗口完整适配工作区，处理负坐标、拔除外屏及小屏 | 纯几何计算不修改领域数据；Alt+F4 保留关闭窗口语义 |

本轮应用 apple-design 审查。参考本地 HIG `accessibility.md` → Mobility 的 iOS/iPadOS 44×44pt 表；`buttons.md` → Best practices：“Make buttons easy for people to use.”；`typography.md` → Supporting Dynamic Type：“Consider adjusting your layout at large font sizes.” Android 使用自身 48dp 触控基准。紧凑图标及 1.8 秒反馈时长属于本产品设计判断，不是 HIG 强制值。颜色沿用主题语义 token，复制入口不新增边框或背景。

## 验证

所有自动化使用隔离测试目录与数据库。原生 Apple 测试使用 Test 配置，未替换正在使用的 Debug 产品。真实模型采用已配置路线 gpt-6.1-terra / Responses，凭据未写入客户端子进程参数或本记录。

| 检查 | 本轮结果 |
| --- | --- |
| Web 静态/单元/构建 | Svelte 零错误零警告；610 项测试通过；生产构建通过 |
| Web 生产 Host 真实长任务 | 6 次真实请求，长任务、一次权限允许、记忆与草稿检查通过；`web-receipt.json` 保留原结果 |
| Web 人机工程浏览器 | 1440×960 深色、390×844 浅色、320×640 深色、900×560 浅色共四例通过；实际复制、重复反馈、搜索焦点、阅读位置、回到底部、视口改变、减少动态效果及横向溢出检查；已复核 320px 深色截图 |
| Mac 原生 | 4 项选定 XCTest 通过，含真实流式回复与切换/定位；1 次真实请求并观察到文本流；构建通过。未通过点击 Mac 复制按钮改写用户剪贴板，因此不将此项记为 Mac 剪贴板实测 |
| iOS Simulator | 1 项真实会话 UI 测试通过，1 次真实请求和文本流；键盘/菜单、冷启动草稿、发送及按钮至少 44pt 和独立命中区域检查通过；已复核最终深色截图 |
| Android JVM/构建 | 689 项测试中 687 通过、2 跳过、0 失败；Debug 测试 APK、lint 通过 |
| Android AVD | 真实回复、停止后继续与 Activity 恢复两项产品 UI 测试通过，3 次真实请求均 HTTP 200；新增紧凑复制测试首次失败，控制 Compose 时钟后单独复测 1 项通过，覆盖深浅色、48dp、无重叠和实际复制反馈。组合验收原失败 receipt 保留，未将其改写为通过 |
| CLI | composer 6 项、terminal_input 6 项通过；产品二进制构建通过。链接器现有大 unwind section 警告仍存在 |
| Windows | TypeScript 构建与 6 项窗口几何单元测试通过。当前 Mac 环境不能执行 Windows 原生系统交互验收 |
| 语法与补丁 | 新浏览器脚本 Node 语法、真实任务脚本 Python 编译、git diff --check 通过 |

### 证据与失败记录

- Web：临时目录 `nola-cross-real-web-wcwa1jlx`。真实长任务通过；首次新浏览器测试因为回答在大视口内无需滚动而未满足测试前置条件，顶层失败 receipt 保留。改用较矮的真实视口后，同一已完成真实会话的四例通过，见 `ergonomics.json` 与截图；没有篡改原失败记录，也没有用合成回复替换真实消息。
- Mac：`nola-cross-real-mac-f9834tnm/receipt.json`、`/tmp/nola-mac-real-memory.AIV3tI`。
- iOS 最终：`nola-cross-real-ios-fnr0h_i4/receipt.json`、`draft.xcresult`；截图导出至 `/tmp/nola-ergonomics-ios-attachments`。
- iOS 首次 `nola-cross-real-ios-3gce2z18` 的唯一失败为测量值 43.99999999999997 与 44 的浮点比较；改为 0.01pt 误差容限，保留原失败证据。
- Android：`nola-cross-real-android-irx46_ul` 保留两个产品 UI 用例日志及三次真实请求；第二轮中的新增复制用例失败导致总 receipt 为 false。最终单独复测 `nola-android-copy-retry-irpfy7_2/receipt.json` 为 true，未发起模型调用。修复的是测试时钟，生产反馈时长未修改；仅重编译测试 APK，沿用本轮已构建的 arm64 Rust 库。`real-android-stop.png` 被 Digital Wellbeing 系统 ANR 弹窗遮挡，不计为干净视觉验收截图；原图保留。
- 单端日志位于 `/tmp/nola-ergonomics-*`；测试脚本为 `scripts/test_real_cross_platform_long_tasks.py`，Web 新浏览器断言为 `web/playwright_runtime/ui_ergonomics_regression.cjs`。

## 未覆盖范围

不将模拟器等同于物理手机；未覆盖全部字号、语言、显示器、浏览器以及长时间后台组合。CLI 终端图片粘贴和全部权限指引不能由此次编辑器测试推定通过。Windows 仍需 Windows 系统上的菜单、剪贴板、缩放、系统关闭和多显示器实测。Mac 复制重复计时目前具有源码/编译及通用原生回归证据，不能以其他平台的剪贴板测试代替。后续按具体复现补充，避免把局部验证写成全量完成。

## 第二轮：键盘输入与档位菜单

继续检查输入、候选操作和面板路径，修改范围为 Mac、Android、Web 的原生档位控件；Windows 使用的共享 Web 页面同步受益。iOS 与 CLI 本轮未发现需要同时改动协议或布局的确定复现，保持上一轮改动，未重复宣称完成全端验收。

- **Mac（高）**：AppKit 的回车自动重复原本继续执行候选接受/提交回调；在未标记文本且无编辑修饰键的普通 Return/数字键盘 Enter 上忽略重复事件。保留 IME 和换行行为。新增原生 NSEvent 回归验证重复事件不提交、不接受第二项，正常首次按下仍分别执行候选接受和发送。
- **Android（中）**：附件粘贴原先只检查 Ctrl/Meta+V，可能截获 Ctrl+Shift+V、Ctrl+Alt+V。增加 Shift/Alt、组合输入、自动重复约束；新增 Compose 键盘事件测试验证前两组不触发附件处理，普通 Ctrl+V 处理一次。
- **Web（中）**：档位列表增加上下循环、Home/End、Escape 焦点恢复；定位不保存档位，Enter/点击才调用选择。带修饰键和 IME 组合事件不被菜单导航抢占。保留原生 details/summary 与按钮语义，不伪造菜单角色。
- **Web 窄屏（高）**：浏览器组件测试实际复现弹层越出 320px 屏幕；改为逻辑端对齐并按可视区域校正水平位移，保留 8px 边缘。滚动到边界不把滚动传给时间线；焦点定位尊重减少动态效果，无额外动画。

设计依据：`keyboards.md` → Best practices 与 `focus-and-selection.md` → Best practices 的 “Avoid changing focus without people’s interaction.”；`scroll-views.md` → Best practices 的 “Support default scrolling gestures and keyboard shortcuts.”。方向键定位、8px 边缘与重复事件策略是产品交互判断。继续沿用现有主题 token，不新增蓝色边框。

### 第二轮验证与限制

- Web：610 项测试通过，Svelte 零错误零警告，生产构建通过。新 `web/playwright_runtime/reasoning_keyboard_regression.cjs` 使用隔离组件夹具，无 Host/凭据/模型请求；1000×700 LTR、320×480 LTR、320×480 RTL 三例通过，覆盖方向键、首尾与循环、Escape、显式选择、禁用状态、点击外部关闭和屏幕边界；已复核 RTL 窄屏截图。证据：`/tmp/nola-reasoning-keyboard/receipt.json`、截图；日志 `/tmp/nola-polish2-web-*-final.log`。
- 第一次新组件浏览器检查因真实水平越界失败，修复后通过；最初将键盘监听放在 details 上产生的无障碍警告已通过改为 summary/按钮监听消除，未加忽略规则。
- Android 首次测试编译缺少实验 API 标记和按键扩展导入；修正仅测试代码，后续构建和设备结果在下方记录。
- Mac 使用 Test 配置与独立 DerivedData。首次调用现有分层脚本时附加的单套件参数未被脚本转发，因此在编译阶段停止该次全量运行；最终使用带设备防护的直接 xcodebuild 明确限制套件。不把被停止的运行记为测试通过。
- 未重新进行真实模型调用：本轮行为可由原生事件与浏览器组件确定验证，不据此替代此前真实长任务结果，也不扩大到 Windows 系统、物理手机或所有输入法实测。

第二轮 Android 最终：`testDebugUnitTest`、Debug 应用与测试 APK、lint 全部构建通过；JVM 689 项中 687 通过、2 跳过。隔离只读 AVD/独立用户中的新键盘用例 `OK (1 test)`，证据 `nola-android-keyboard-d74vohb1/receipt.json`；没有模型请求或修改用户剪贴板。Rust 源码未变，沿用上一轮已构建的 arm64 原生库。

第二轮 Mac 最终：`MacComposerRecoveryStateTests` 三项通过、零失败，含真实 AppKit 回车与数字键盘 Enter 事件；应用和测试目标构建通过。证据 `/tmp/nola-polish2-mac.xcresult`、`/tmp/nola-polish2-mac-final.log`。测试保持 Test 配置；未替换正常 Debug 应用。上述最后状态已核对，Node 语法和 `git diff --check` 通过。既有 headermap/大前端 chunk 构建警告仍保留，本轮未将其隐藏。
