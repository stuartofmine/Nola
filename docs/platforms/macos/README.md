# macOS

实现位于 [NolaMac](../../../NolaMac/)，使用 SwiftUI/AppKit；不是 CLI 包装界面。

- [信息架构](UI_INFORMATION_ARCHITECTURE.md)、[应用设计](APP_DESIGN.md)。
- [Apple 检查器](../apple/CONVERSATION_INSPECTOR.md)、[共享交互](../../design/README.md)。
- [宿主机配对与执行](../../design/shared/HOST_CONNECTION_WORKFLOW.md)、[客户端边界](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。

使用 `./scripts/test_mac_layers.sh full` 执行 Mac 测试，必须使用 Test 配置。正常 Debug 制品保持项目配置的签名身份；不要把未签名 XCTest 输出写入 Products/Debug，避免替换用户正在使用的应用或破坏 Keychain 身份。

本轮同等 200 步远端长任务尚未在 Mac 原生完成，详见[实际验收范围](../../quality/VALIDATION_SCOPE.md)。
