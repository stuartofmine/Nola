# Apple 共用设计

[Shared](../../../Shared/) 提供 Apple 两端共享呈现与协议实现，平台应用仍保留自己的导航、系统权限和生命周期。

- [Apple 会话检查器](CONVERSATION_INSPECTOR.md)。
- [macOS](../macos/README.md)、[iOS / iPadOS](../ios/README.md)。
- [客户端投影契约](../../architecture/模块/13-客户端投影/README.md)、[共享视觉系统](../../design/shared/DESIGN_SYSTEM.md)。

相同 Swift 类型或共享渲染逻辑不代表两端原生窗口、系统手势、授权或长任务已分别验收。
