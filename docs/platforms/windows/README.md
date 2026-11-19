# Windows

实现位于 [windows](../../../apps/windows)，Tauri 2 使用系统 WebView2 承载共享 Web 界面，通过随包的 Rust Host 访问本地会话。安装包使用当前用户范围的 NSIS。

- [Web 页面与构建](../web/README.md)、[客户端及窗口边界](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [软件更新](../../design/shared/SOFTWARE_UPDATE_EXPERIENCE.md)、[更新架构](../../architecture/总体设计/SOFTWARE_UPDATE_ARCHITECTURE.md)。

Windows 制品需要原生 Windows 工具链。安装后用 `npm run test:e2e` 验证 WebView2、会话和重启持久化。其他测试边界见[实际范围](../../quality/VALIDATION_SCOPE.md)。
