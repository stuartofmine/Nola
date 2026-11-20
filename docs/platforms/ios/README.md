# iOS / iPadOS

实现位于 [Nola](../../../apps/ios)，SwiftUI 通过共享 Rust bindings 接入 embedded Application；设备权限由平台管理，配对远端 Host 不授予设备权限。

在本机对话里可以说“找到 GitHub 上 Nature Skills 里做论文精读卡的 Skill，装到这部手机”。也可以直接贴仓库或 Skill 目录链接。安装并启用后，在新的本机会话中即可调用。想给电脑使用时，选择已配对的 Host 并在该对话中提出安装请求。两处的 Skill 设置分别保存。[安装位置与资源范围](../../architecture/模块/11-Skill插件与MCP/README.md)

输入 `@` 后直接在键盘上方点选专家，名字保留在正文里。点已发送的名字可查看该次提及和对应专家历史。从只读子会话继续时，会回到主会话草稿。远程镜像可读取和复制，编辑重发在保存原会话的 Host 上完成。[消息内的专家提及](../../design/shared/EXPERT_MENTIONS.md)

- [Apple 共用检查器](../apple/CONVERSATION_INSPECTOR.md)、[客户端导航、模型与调用边界](../../architecture/模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。
- [会话交互](../../design/shared/CONVERSATION_UI_DESIGN.md)、[宿主机流程](../../design/shared/HOST_CONNECTION_WORKFLOW.md)。
- 测试前用 `xcrun simctl list devices available` 检查可用模拟器；通过 `./scripts/run_client_tests.sh` 运行，使用隔离数据和受控 XCTest 克隆。不轮询锁定的物理设备。

手机 OS 不保证本机永久后台执行；远端 Host 任务的权威与客户端生命周期分开。[原型](../../design/prototype/README.md)不验证原生手势、系统权限或物理设备后台行为；实际范围见[质量记录](../../quality/VALIDATION_SCOPE.md)。
