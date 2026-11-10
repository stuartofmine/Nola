# iOS / iPadOS

实现位于 [Nola](../../../Nola/)，SwiftUI 通过共享 Rust bindings 接入 embedded Application；设备权限由平台管理，配对远端 Host 不授予设备权限。

- [Apple 共用检查器](../apple/CONVERSATION_INSPECTOR.md)、[客户端导航、模型与调用边界](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [会话交互](../../design/shared/CONVERSATION_UI_DESIGN.md)、[宿主机流程](../../design/shared/HOST_CONNECTION_WORKFLOW.md)。
- 测试前用 `xcrun simctl list devices available` 检查可用模拟器；通过 `./scripts/run_client_tests.sh` 运行，使用隔离数据和受控 XCTest 克隆。不轮询锁定的物理设备。

手机 OS 不保证本机永久后台执行；远端 Host 任务的权威与客户端生命周期分开。[原型](../../design/prototype/README.md)不验证原生手势、系统权限或物理设备后台行为；实际范围见[质量记录](../../quality/VALIDATION_SCOPE.md)。
