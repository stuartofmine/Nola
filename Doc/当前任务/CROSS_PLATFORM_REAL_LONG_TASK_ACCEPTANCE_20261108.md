# 跨平台真实模型长任务验收

日期：2026-11-08。沿用当前默认模型 `gpt-6.1-terra` / Responses。范围为 CLI、macOS、iOS Simulator、Android AVD 和真实 Chrome 中的 Web；模拟器运行不等于手机真机验收。

## 已确认并修复的产品错误

1. Mac 本地 Agent 在没有选择 agent reasoning tier 时发送 `reasoning_tier: null`。Application 当前合同拒绝这个值，请求在发出模型调用之前退出，UI 进入结果待确认。现在只在有值时发送此字段；没有选择时省略。回归检查直接断言 JSON 字段缺席，不再用 `as? String == nil` 把 NSNull 当作正确。
2. Android JNI 正确加载了当前 Rust 制品，但 Kotlin 可用性检查仍要求 Application Service 协议 53，实际制品导出 54，导致整个 Application runtime 被拒绝。现在只接受 ABI 1 / 协议 54，同时测试拒绝 52、53、55 和错误 ABI；没有增加旧协议回退或改写数据库。

3. Android 取消后的 canonical seal 已提交，但 generation job 尚在物理收尾时，UI 提前显示可发送。一次真实点击被 guard 拒绝并保留草稿，用户需要重试。现在把 execution teardown 的本地状态投影到 composer：收尾期间保留草稿编辑、禁用发送并显示现有本地化原因，job 真正完成后恢复；不提前释放执行 lease，也不重新提交用户请求。增加独立 owner 与封口后延迟收尾的回归用例。

4. iOS 紧凑布局同时创建自定义底部导航与隐藏的系统 TabView。iOS 27 的隐藏 UITabBar 仍保留 `CAMatchMoveAnimation(duration=inf)`；XCTest 在每次键盘事件前等待 UI 静止，端点输入和下一字段聚焦分别耗时约 121 秒。采样时应用主线程正常等待事件，测试线程停在动画静止等待。现在使用按标签缓存 hosting 根、仅挂载当前页的容器，紧凑布局不创建系统 TabView；常规宽度保留系统导航。未关闭 XCTest 静止等待、未禁用全局动画。

5. iOS 提交用例在调用 execution 的 Stop 前先取消提交 Task。消费者的 stream termination 能抢先触发 observer detach、退役 permit，导致 native coordinator 来不及持久化 interrupt；UI 随后显示错误/可发送，数据库仍是 running，下一轮被准入检查拒绝。现在提交 Task 的退役回调交给 execution，原生路径先提交 Core cancellation，再退役观察和消费者；无原生 coordinator 的准备阶段及远程路径仍保留各自取消流程，不把 EOF 当终态。真实长任务复测已观察到 cancelled seal 和成功的后续回答。

## 场景和结果

| 平台 | 实际覆盖 | 结果 |
| --- | --- | --- |
| CLI | 上轮六类：多文件开发、依赖链、冷启动、记忆隔离、权限等待、实际压缩和后台任务 | [117 次真实请求及详细证据](CLI_REAL_LONG_TASK_ACCEPTANCE_20261108.md) |
| Mac | 真实 SwiftUI 长回复（60 条中文建议）；生成中切换会话、返回原会话、末尾定位、其他会话草稿、canonical 成功 seal | 通过；1 次真实请求，另 3 项推理参数测试通过；共 4 XCTest，0 跳过 |
| Web | 真实 Chrome + Rust Host；页面仅允许本次；30 秒命令、30 步进度、只执行一次；窄屏切换会话、关闭重开标签页、英文追问和其他会话草稿 | 通过；6 次真实请求，独立文件结果 total=465 / steps=30，starts 只出现一次 |
| iOS | 草稿/键盘/工具面板/冷启动，以及多轮、长回复导航、取消、再发送和新会话隔离 | 两个独立场景均通过，0 跳过；共 7 次真实请求。草稿 Run 的结果/seal 另经数据库一致性核对 |
| Android | 真实 MainActivity 流式文本、持久化及 Activity 恢复；真实 OS 后台/返回、Stop、下一轮和 Activity 重建 | 两个真实 UI 场景均通过；3 次真实请求。后台状态 STARTED，返回时仍在运行；停止后回答 AFTER_STOP=703，Activity 重建保留结果。数据库核对 answered、cancelled→answered 三个 Run |

Mac 确认了生产 Application 返回的 terminal 和数据库不可变 seal 一致，不把“UI 空闲”当作完成。Web 除页面文本外核对隔离项目文件和 canonical Run。Android 原始 UI 用例曾已通过而外层 gate 失败，原因是 observer 没读取 Responses 的 input；没有把这个误判记成产品错误。

## 测试入口修正

- iOS 测试跟随当前 compact bottom navigation、`chat.navigation.back` 和会话工具面板，移除寻找旧系统 TabBar/NavigationBar/更多菜单的假设。
- Android 在实际 UI 上确认当前模型能力，使用当前未合并语义树选择模型，等待输入框真正启用后输入。后台步骤从目标测试用户的 Context 启动系统 Home；UiAutomation 全局 Home 可能作用到主用户，不能用它的成功返回冒充目标 Activity 已后台。
- provider observer 在显式开启采集时读取 Responses input 和当前 messages 文本块；不采集图片内容，保留长度限制，默认不开启请求文本采集。新增 Responses 文本/图片块与长度边界检查。
- Mac gate 分开 XCTest 数和必须成功的模型 Run 数，避免三个无模型调用的参数回归用例被计成三个模型 Run。seal 验证保持不变。
- 初次错误、人工中止和重测前的 receipt 均保留；不将它们与最后通过场景合并成“全部尝试通过”。

## 复现与隔离

新入口：[test_real_cross_platform_long_tasks.py](../../scripts/test_real_cross_platform_long_tasks.py)。可用 `--platform web|mac|ios|android|all`；默认 all 依次运行，每次使用真实模型并产生费用。iOS 的 `--ios-case all`（默认）覆盖 draft、lifecycle、setup 和 keyboard；可单独选择其中一项，`both` 仅运行 draft 与 lifecycle。每项 UI 通过后还须验证至少一个 answered canonical Run 及其 result/seal 一致性。

```sh
python3 scripts/test_real_cross_platform_long_tasks.py --platform all
PYTHONPATH=scripts:scripts/e2e python3 -m pytest -q \
  scripts/tests/test_provider_probe_streaming.py \
  scripts/e2e/test_cross_platform_relay_safety.py \
  scripts/e2e/test_owned_android_device_selection.py \
  scripts/e2e/test_long_task_fixture_safety.py \
  scripts/e2e/test_real_file_permission_audit.py
```

上述 Python 检查 30 项通过；Android JVM 22 项通过（包括 2 项协议合同，以及收尾竞态、运行状态、房间与草稿归属），lintDebug 通过。另做 Python 编译、Node/Bash 语法及 diff 空白检查。Web 另有 610 项单元测试通过，Svelte 检查 0 错误/0 警告，前端构建通过。它们是辅助检查，不能算真实模型调用。

本轮最终通过的 Mac/Web/iOS/Android 场景共使用 17 次真实请求，不包含前期失败和重跑。取消用例的 HTTP 流因客户端主动停止而中断，与 cancelled seal 一致，不将其伪记为正常生成完成。

真实 credential 只保留在透明 relay 内存，原生客户端只连接测试 relay；原始用户配置、用户数据库和凭据未改写。Mac 使用独立 `Products/Test`。Android 使用本轮启动的 read-only AVD overlay 和独立次要用户，runner 显式限定所拥有的 emulator serial（缺失就拒绝，不改选连接的手机），测试结束退出拥有的模拟器，不清除用户日常安装。iOS 只使用已列出的 Simulator 和 UUID 测试命名空间，不连接物理设备。拒绝的 metadata 请求不转发、不算 provider 请求。

通过的证据根目录（本机临时目录可能被系统清理）：

- 汇总：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cross-platform-summary-20261108.json`。
- Mac：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cross-real-mac-9ur9prkz`；XCTest / UI / seal 证据在 `/tmp/nola-mac-real-memory.nmju2Y`。
- Web：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cross-real-web-qf0mift1`。

- iOS 长任务：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cross-real-ios-3alrzlw9`；草稿：`nola-cross-real-ios-7z4317sd`（同一临时父目录），包含导出的真实截图、XCTest summary 和 canonical-audit。
- Android：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cross-real-android-yrr67d_c`，包含两个 instrumentation 输出、真实请求记录、后台/返回证明、UI 语义树和最终 Store audit。首场景截图与持久化审计在该目录 android-streaming.log 记载的独立 UI 证据目录。第二场景的截图导出文件名曾写错，已经保留 missing 诊断并修正后续 runner；不把这个错误文本文件当 PNG 截图。

iOS 配置表单卡顿已在后续修复并单独复测：未预置模型，从实际表单输入端点和模型；切到聊天再返回，导航栈和两个字段保持，随后保存并收到真实 `MODEL_SETUP_OK`。表单操作约 16 秒，端点输入约 1 秒；完整 XCTest 42.3 秒，0 跳过。测试增加 60 秒表单预算，数据库另核对 answered/result/seal 一致。证据：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cross-real-ios-kh253lh9`，含分段耗时、两张实际截图和 canonical-audit。此前仅验证输入/保存的修复后场景为 `nola-cross-real-ios-1l2s8xm2`；诊断中止和初次编译失败均按失败保留。草稿/长任务用例仍使用当前 Application/Store 合同预置模型，配置表单由独立 setup 场景负责；没有伪造回复或状态。

iOS 冷启动回归的截图检查另发现采集时序错误：截图中模型按钮在 y=700，随后 AX 返回 y=758，旧检查拿导航/底部 inset 动画前的像素配动画后的坐标，误报 range=0。修正为五秒内要求坐标稳定且实际像素有对比度；仍保留 >40 的阈值，不跳过检查、不关闭动画。失败也先保存截图、界面树和区域诊断。诊断失败证据保留在 `nola-cross-real-ios-vb9mnyop`、`nola-cross-real-ios-wwcf20do` 与 `nola-cross-real-ios-0rvppz73`，不计入通过场景。

## iOS 同日续修复验

生产改动限于紧凑标签宿主和原生 Stop 的消费者退役顺序；未修改生产 JSON 解码类型、未放松模型准入、未关闭 XCTest 静止等待或全局动画。

- 模型表单：最终场景 `nola-cross-real-ios-166myt0m`，实际输入后跨标签返回、字段与导航保留、保存、真实回复均通过；表单约 18 秒，端点输入约 1 秒，XCTest 49.9 秒，answered/result/seal 核对通过。
- 长任务：`nola-cross-real-ios-szmpz810`，6 次真实请求，0 跳过。原会话 5 个 Run 为 answered / answered / answered / cancelled / answered，新会话 1 个 answered；均具备一致的 canonical result/seal。HTTP 取消流不计作正常完成的文本流。
- 键盘与切页：`nola-cross-real-ios-80mbg1lf`，1 次真实请求，0 跳过；键盘前方输入框、拖动收键盘、切标签后的草稿与历史保留通过，数据库核对通过。
- 冷启动草稿最终复验：`nola-cross-real-ios-42n8bpm3`，1 次真实请求，0 跳过；中文多行草稿、工具面板、原生侧滑、跨会话隔离、进程重启及真实发送通过，answered/result/seal 核对通过。
- 单元回归：`/tmp/nola-ios-compact-final-unit.xcresult`，19 项通过，0 失败/0 跳过；覆盖容器保留与单页挂载、原生返回、取消授权/物理停止/观察 detach 边界、会话隔离与并发 HTTP 执行。Python 30 项检查另通过。

以上四项独立 UI 场景共 9 次真实请求，4 XCTest / 0 失败 / 0 跳过，均有数据库审计；汇总为 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-ios-continued-repair-summary-20261108.json`。临时根目录均位于 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T`；系统可能清理它们。

并发单测最初 18/19 通过。边界附件确认实际拒绝为“Selected model is not enabled for chat”，测试 fixture 未声明当前 Core 所需的 chat 能力与 automatic reasoning；补齐当前 profile 合同并在等待 HTTP continuation 前验证准入后通过。XCTest 展示的中间解码错误与实际回调不一致，以回调附件定位；没有据此修改业务字段。实际运行宿主已核对为当前代码。临时阶段/回调探针全部撤掉，诊断失败结果保留，不计为通过；fixture 是确定性单测 HTTP peer，不计入真实 provider 请求。

本轮不能关闭：所有 provider、物理手机、图片粘贴、全部主题/字号/无障碍组合、断网/限流和多小时运行。报告中未列为通过的项目继续保留边界。
