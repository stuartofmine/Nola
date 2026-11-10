# CLI

实现位于 [cli](../../../cli/)，提供终端客户端、Agent Host 和本地 HTTP 服务。

- [CLI 使用与配置](../../../cli/README.md)、[模型配置](../../architecture/模块/08-模型配置与调用/README.md)。
- [执行与恢复](../../architecture/模块/05-执行与恢复/README.md)、[权限与平台](../../architecture/模块/07-权限与平台/README.md)。
- 构建使用 `cargo build --locked -p nola_cli -p nola_service --bins`；实际命令与交互按键以 `nola --help` 和终端帮助为准。

图像粘贴取决于终端/系统与模型能力。Goal、会话、后台作业和具体操作授权分别有自己的所有者，不靠自然语言关键词选择工具或授予权限。终端原型是布局示意，不添加命令。[验收范围](../../quality/VALIDATION_SCOPE.md)不会用浏览器原型关闭 CLI 的未测交互。
