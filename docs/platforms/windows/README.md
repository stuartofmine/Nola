# Windows

实现位于 [windows](../../../apps/windows)，Tauri 2 使用系统 WebView2 承载共享 Web 界面，通过随包的 Rust Host 访问本地会话。安装包使用当前用户范围的 NSIS。

桌面主窗口连接本应用启动的 Host；在浏览器中访问其他机器的 Host 地址。 [连接宿主机的操作与边界](../../design/shared/HOST_CONNECTION_WORKFLOW.md)。

Tauri 与 Web 共用消息编辑器。在句中输入 `@`，用上下键和回车选人，发出后可从原句中的名字打开专家历史。[消息内的专家提及](../../design/shared/EXPERT_MENTIONS.md)

- [Web 页面与构建](../web/README.md)、[客户端及窗口边界](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [软件更新](../../design/shared/SOFTWARE_UPDATE_EXPERIENCE.md)、[更新架构](../../architecture/总体设计/SOFTWARE_UPDATE_ARCHITECTURE.md)。

Windows 制品需要原生 Windows 工具链。安装后用 `npm run test:e2e` 验证 WebView2、会话和重启持久化。其他测试边界见[实际范围](../../quality/VALIDATION_SCOPE.md)。

## 在 Windows 构建

安装 Node.js 24、Rust MSVC 工具链、Visual Studio C++ 构建工具、CMake 和 Git。在仓库根目录运行以下命令。

```powershell
cd apps/windows
npm ci
npm run dist:x64
```

x64 CPU 安装器位于 `apps/windows/release/x64-cpu/`。安装程序按当前用户安装。首次启动会在文档目录下创建 `Nola Workspace`。

## 验证已安装版本

在使用英文界面的独立 Windows 测试账号中，设置 `NOLA_INSTALLED_DESKTOP` 指向已安装的 `nola-desktop.exe`，在 `apps/windows` 下运行 `npm run test:e2e`。测试使用当前桌面和剪贴板，在真实窗口里创建会话、输入草稿并选择文本附件。退出重开后，会核对会话标识、草稿原文和附件名称。
