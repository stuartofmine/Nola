# CLI

在交互终端里输入 `@` 后可用上下键选专家，回车把名字插在光标处。按 Esc 保留输入的普通文字，按退格删除整个已选名字。`/edit` 打开最近一条用户消息，`/edit <消息 ID>` 打开指定消息。重新发送仍在同一会话内，按 Esc 或输入 `/cancel-edit` 退出编辑。

实现位于 [cli](../../../apps/cli)，提供终端客户端、Agent Host 和本地 HTTP 服务。

- [CLI 使用与配置](../../../apps/cli/README.md)、[模型配置](../../architecture/模块/08-模型配置与调用/README.md)。
- [执行与恢复](../../architecture/模块/05-执行与恢复/README.md)、[权限与平台](../../architecture/模块/07-权限与平台/README.md)。
- 构建使用 `cargo build --locked -p nola_cli -p nola_service --bins`；实际命令与交互按键以 `nola --help` 和终端帮助为准（支持 `--model` / `-m`、`--permission-profile`、`-y` / `--yes`、`nola skills`、`nola mcp`、`nola memory`、`nola version` 与 9 种界面语言 `zh` / `zh-hant` / `en` / `de` / `es` / `fr` / `ja` / `ko` / `ru`）。

图像粘贴取决于终端/系统与模型能力。Goal、会话、后台作业和具体操作授权分别有自己的所有者，不靠自然语言关键词选择工具或授予权限。终端原型是布局示意，不添加命令。[验收范围](../../quality/VALIDATION_SCOPE.md)不会用浏览器原型关闭 CLI 的未测交互。


## 网页检索与代理 DNS

`web_search.limit` 决定返回和解析的结果数量。无可用正文证据的候选保留在完整诊断报告中，不提升为可引用的模型来源。

`web_fetch` 在每次连接和重定向前校验网址与解析地址，并固定已校验的公网地址。系统 DNS 若仅返回 `198.18.0.0/15` Fake-IP 别名，会通过固定的 Google Public DNS HTTPS 接口查询真实地址，再执行相同的公网校验；不会直接连接 Fake-IP、内网、回环或元数据地址。该恢复查询仅发送域名，不发送网页路径、查询参数或凭据，并关闭 EDNS 客户端子网信息。设置 `NOLA_WEB_FAKE_IP_RECOVERY=0` 可关闭公共解析恢复；失败提示会指引为代理配置真实 IP 解析或改用已连接浏览器。

## 从 GitHub 安装 Skill

在对话中输入“帮我找到 Nature Skills 里做论文精读卡的 Skill，安装并启用”。也可以直接粘贴公开仓库、Skill 子目录或 `SKILL.md` 的 GitHub 链接。遇到含多个 Skill 的合集，可以指定用途或选择其中一个。安装完成后，新会话可以直接调用该 Skill，Web 设置里的 Skill 列表可以查看和切换启用状态。

终端也支持以下操作。

```shell
nola skills find "nature skills"
nola skills find https://github.com/Yuan1z0825/nature-skills
nola skills install https://github.com/Yuan1z0825/nature-skills/tree/main/skills/nature-paper-card
nola skills disable nature-paper-card
nola skills enable nature-paper-card
```

安装器把选定目录和清单 `manifest.yaml` 的 `always_load` 引用文件固定到同一次 Git 提交，保留仓库里的相对位置。禁用后保留包内的脚本、模板和资料，再次启用即可恢复。安装发生在执行工具的 Nola Host 上。涉及额外软件依赖、凭据或 MCP 服务的 Skill，仍需分别配置这些资源。
