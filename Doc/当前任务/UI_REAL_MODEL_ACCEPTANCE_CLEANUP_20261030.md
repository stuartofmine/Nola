# UI 测试清理与真实模型验收（2026-10-30）

## 唯一验收标准

UI 验收必须通过产品真实入口提交任务，由配置的真实模型处理，并检查实际可见的结果及持久化状态。仅发送请求不算完成；响应不能由本地脚本、固定 JSON、模拟流或预置对话代替。

每次验收记录平台、页面与操作、模型/服务标识（不含密钥）、实际请求证据、可见响应、最终任务状态。流式/思考显示应观察响应到达过程；停止、恢复、编辑重发、多专家场景应记录完整真实调用链和恢复结果。读回历史可作为该链的一步，不能单独充当模型验收。

未配置服务、服务不可达、测试被跳过或没有实际执行用例，应报告未验证，不能写成通过。构建、静态检查、Core 单元测试均不能替代真实 UI 使用证据。界面问题仍以逐页对照原型、实际点击和完成任务为准。

## 本次清理

- 删除离线控件存在性、布局/截图、预置对话、手写模拟流、假 HTTP 响应的 UI 验收及独立无模型验收入口。
- 第一轮删除 193 个文件、17664 行，包括 Web 交互 harness、生成的 UI 测试资源以及 iOS、macOS、Android 的离线 UI 套件。随后删除 Android 无模型远端 UI 验收脚本，并从混合套件继续剥离离线 UI 方法。
- iOS 保留真实 Host、真实 Provider、专家与会话生命周期链；停止辅助操作留在真实停止后重发链内。移除产品启动时注入假对话、假 Host、假资料的专用分支。协议/恢复单元测试使用的确定性 fixture 移入 NolaTests，不再装入产品。
- Android 保留真实模型对话、A2UI、编辑重发、远程任务、专家补全和权限工作流。真实链所需的隔离配置/清理设施保留，但不计为验收。Room trace 持久化测试移到 data/repository，删除同文件内假 Compose 流式测试。
- macOS 保留真实产品模型工作流，移除独立离线视觉层和混合套件内的注入式渲染测试；脚本入口改为 real-ui。删除快速评估中的假长历史 UI 性能项。
- CLI 删除 PTY 脚本模型 UI 套件及其 7 个分片，共 8 个文件、14,513 行；真实模型终端验收脚本保留。
- Web 保留真实 Provider 浏览器流程、透明请求记录代理及真实专家工作流；移除独立模拟 UI harness 的构建脚本、配置和资源。
- 保留 Core 协议、权限、存储、解析及非 UI 状态契约测试；它们单独报告，不作为 UI 已验证的结论。
- 更新重命名后的 XCTest 选择器，删除失效运行入口及 105 条仅用于已删除测试的本地化文本。

## 静态与构建检查（不能替代真实模型验收）

- Web：svelte-check 0 错误、0 警告，生产构建通过。
- Android：compileDebugAndroidTestKotlin 通过。
- Swift：清理涉及的 13 个文件语法解析通过；Apple 全量构建未通过确认：iOS 的 NotificationStoreTests.swift（x86_64）和 macOS 的 MacCodeReferencePreview.swift、MacJournalHistoricalPartView.swift 命令出现“退出码 0 但没有输出”的构建异常；停止全量构建及 arm64 重试，未宣称通过。
- Shell 语法、Python 语法、删除源文件及失效选择器引用检查通过；Cargo 元数据中已无脚本模型 PTY 目标。
- macOS real-ui 转入已有真实 Provider 验收器，检查实际请求证据、准确执行数量、零跳过。Web 显式开启真实模型验收后，缺少配置直接失败。

## 保留入口

- iOS：NolaUITests/IOSRealHostChatTests.swift、IOSRealProviderChatTests.swift、IOSRealConversationLifecycleTests.swift、IOSRealExpertAcceptanceTests.swift、IOSAgentRoomRealModelTests.swift。
- macOS：NolaMacTests/MacRealProductUIEndToEndTests.swift；scripts/test_mac_layers.sh real-ui（转入现有真实 Provider 产品工作流，要求实际请求证据、准确用例数和零跳过）。
- Android：ui 包内实际模型工作流及其隔离配置辅助设施。
- Web：web/tests/test_real_llm_web_agent_e2e.py、scripts/test_web_expert_mentions.py；real_workspace_protocol_e2e.mts 是真实协议集成，不是浏览器点击证据。

## 变更审计

清理前的工作区已有大量未提交修改；本次没有回退这些修改，也没有清理用户数据库、模型配置或凭证。删除前文件内容和逐文件摘要暂存于 /tmp/nola-ui-test-cleanup-20261030，便于核对；不纳入版本库。

本次清理的编译检查结果另行报告。没有新跑真实模型 UI 链，不将本次删除后的编译成功表述为产品验收通过。

## GitHub Actions 同步清理

见 [.github/README.md](../../.github/README.md)。移除失效 Mac harness、重复 release 构建及失效产物上传；真实工作流排除 3 个非模型场景，保留五端覆盖与完整执行检查。静态/证据/Rust 契约独立报告，不算 UI 验收。未删除远程历史记录或触发远程模型调用。
