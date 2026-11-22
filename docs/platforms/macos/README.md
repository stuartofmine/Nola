# macOS

在「模型 → 本地模型」查看本机硬件，选择引擎和下载源，留空模型名即可获取推荐，也可以输入指定模型。推荐优先考虑本机能开到 128K 的方案，同等窗口下优先选择 Unsloth 量化。首次选择引擎时会检查是否可用。已有 Homebrew 的 Mac 可在页面内安装引擎，Apple 芯片 Mac 也可直接用内置 MLX。核对模型大小、预计内存和建议窗口后，确认下载。[本地模型推荐、下载与启动](../../architecture/模块/08-模型配置与调用/DESKTOP_LOCAL_MODELS.md)

实现位于 [NolaMac](../../../apps/macos)，使用 SwiftUI/AppKit；不是 CLI 包装界面。

在输入框的光标处输入 `@`，可以用键盘或鼠标选人。编辑已发送的文本消息会恢复原句和当时选中的专家，同一主会话接受替换消息后回溯旧回复。远程镜像可读取和复制，编辑重发在保存原会话的 Host 上完成。[消息内的专家提及](../../design/shared/EXPERT_MENTIONS.md)

- [信息架构](UI_INFORMATION_ARCHITECTURE.md)、[应用设计](APP_DESIGN.md)。
- [Apple 检查器](../apple/CONVERSATION_INSPECTOR.md)、[共享交互](../../design/README.md)。
- [宿主机配对与执行](../../design/shared/HOST_CONNECTION_WORKFLOW.md)、[客户端边界](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。

使用 `./scripts/test_mac_layers.sh full` 执行 Mac 测试，必须使用 Test 配置。正常 Debug 制品保持项目配置的签名身份；不要把未签名 XCTest 输出写入 Products/Debug，避免替换用户正在使用的应用或破坏 Keychain 身份。

本轮同等 200 步远端长任务尚未在 Mac 原生完成，详见[实际验收范围](../../quality/VALIDATION_SCOPE.md)。
