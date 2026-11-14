# CLI

实现位于 [cli](../../../apps/cli)，提供终端客户端、Agent Host 和本地 HTTP 服务。

- [CLI 使用与配置](../../../apps/cli/README.md)、[模型配置](../../architecture/模块/08-模型配置与调用/README.md)。
- [执行与恢复](../../architecture/模块/05-执行与恢复/README.md)、[权限与平台](../../architecture/模块/07-权限与平台/README.md)。
- 构建使用 `cargo build --locked -p nola_cli -p nola_service --bins`；实际命令与交互按键以 `nola --help` 和终端帮助为准。

图像粘贴取决于终端/系统与模型能力。Goal、会话、后台作业和具体操作授权分别有自己的所有者，不靠自然语言关键词选择工具或授予权限。终端原型是布局示意，不添加命令。[验收范围](../../quality/VALIDATION_SCOPE.md)不会用浏览器原型关闭 CLI 的未测交互。


## 网页检索与代理 DNS

`web_search.limit` 决定返回和解析的结果数量。无可用正文证据的候选保留在完整诊断报告中，不提升为可引用的模型来源。

`web_fetch` 在每次连接和重定向前校验网址与解析地址，并固定已校验的公网地址。系统 DNS 若仅返回 `198.18.0.0/15` Fake-IP 别名，会通过固定的 Google Public DNS HTTPS 接口查询真实地址，再执行相同的公网校验；不会直接连接 Fake-IP、内网、回环或元数据地址。该恢复查询仅发送域名，不发送网页路径、查询参数或凭据，并关闭 EDNS 客户端子网信息。设置 `NOLA_WEB_FAKE_IP_RECOVERY=0` 可关闭公共解析恢复；失败提示会指引为代理配置真实 IP 解析或改用已连接浏览器。
