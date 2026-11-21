# CLI

## 配置文件

在项目目录运行 `nola config`。它按启动时的实际顺序列出配置文件，显示文件运行设置、模型目录的来源，以及这台宿主已经保存的默认模型。对话中可输入 `/config` 查看同一份信息。它显示的是配置文件和宿主默认值，已有会话的绑定、命令行参数及环境变量仍按各自的运行规则生效。

| 操作 | 命令 | 作用 |
| --- | --- | --- |
| 查看 | `nola config` | 查看用户与项目配置的叠加顺序，对照文件声明和宿主默认值 |
| 编辑 | `nola config edit [文件]` | 用 `VISUAL` 或 `EDITOR` 打开文件。省略路径时打开顺序最靠后的现有配置；首次使用时打开用户的 `.nola/config.jsonc`。关闭编辑器后检查文件；下次启动时自动应用 |
| 检查 | `nola config check [文件]` | 省略路径时检查整组启动配置，指定路径时仅检查该文件 |
| 预览 | `nola config apply [文件] --dry-run` | 显示将保存的连接、模型和宿主默认值。凭据和已有连接冲突在实际应用时核对 |
| 立即应用（可选） | `nola config apply [文件]` | 保存声明的连接和模型，并设置这台宿主的默认模型。省略路径时使用当前项目生效的模型目录 |
| 查看示例 | `nola config example` | 输出可编辑的 JSON 模板。把示例地址和模型 ID 换成实际值；有密钥的连接可使用 `api_key_env` 引用环境变量 |

**选文件的规则。** 启动先读取用户目录的 `.nola/config.jsonc`、`.nola/config.json`，再从最近的 Git 仓库根目录到当前项目逐层读取。每层依次读取 `.nola/config`、`nola`、`.nola/current_config`，每组都是 JSONC 在前、JSON 在后。仓库外只读取用户目录和当前项目。后层可以替换标量或整个模型目录，权限等集合沿用其各自的合并规则。只设置语言的文件不会取代前一层的模型目录。显式文件路径支持相对路径；`--project` 改变项目发现范围，显式相对路径从当前工作目录解析。

**保存后直接启动。** CLI 和 Web 宿主在启动时读取运行设置，并将生效文件的连接、模型与默认选择一次性保存。项目文件可以设置整台宿主的默认模型。默认选择必须引用同一文件内的连接和模型 ID。模型目录校验失败时会显示出错文件，已有默认模型仍可用于新对话。首次启动没有可用默认模型时，修正文件后重试。已有会话保留原模型。若整个文件无法解析，启动会停止并显示文件位置。

项目连接首次使用环境变量密钥时，请在用户配置中声明相同地址、协议和环境变量名；以后可以复用该连接已保存的凭据。

原有的 `nola model apply-config [文件]` 保留原来的 JSON 回执。省略路径时仍按用户目录中的 `config.json`、`config.jsonc` 二选一。新命令支持 `--json`，方便脚本读取结果。

## 对话操作

输入 `/help` 查看常用操作，`/help all` 列出全部命令，`/help edit` 查看单个命令的用法。输入命令前缀后，用 ↑、↓ 浏览带说明的候选。Tab 将选中项填入输入栏，Enter 执行，Esc 收起并保留原文。进入 `/allow` 后，用方向键或数字选择权限模式，按 Enter 应用，Esc 取消。

输入 `@` 后用 ↑、↓ 选专家或工作区文件。第一次按 Enter 将选中的名字或路径插入草稿，可以接着打字，再按 Enter 发送。Esc 收起候选并保留文字。浏览历史输入后，按 ↓ 回到末尾可找回原草稿和光标位置。

长文本粘贴到输入栏后会折叠为带字数的标记。等长的粘贴按编号区分，按方向键可以跨过整段，退格可以删除整段。发送时按草稿中的位置还原正文。

`/edit` 打开最近一条用户消息，`/edit <消息 ID>` 打开指定消息。重新发送仍在同一会话内，按 Esc 或输入 `/cancel-edit` 退出编辑。

提供宿主机服务与连接远端 Web 桥是两条独立命令路径。可用 `nola web --connect-host --help` 查看受信助手、Host 身份和令牌输入要求。 [连接宿主机的操作与边界](../../design/shared/HOST_CONNECTION_WORKFLOW.md)。

实现位于 [cli](../../../apps/cli)，提供终端客户端、Agent Host 和本地 HTTP 服务。

- [CLI 使用与配置](../../../apps/cli/README.md)、[模型配置](../../architecture/模块/08-模型配置与调用/README.md)。
- [执行与恢复](../../architecture/模块/05-执行与恢复/README.md)、[权限与平台](../../architecture/模块/07-权限与平台/README.md)。
- 构建使用 `cargo build --locked -p nola_cli -p nola_service --bins`；实际命令与交互按键以 `nola --help` 和终端帮助为准（支持 `--model` / `-m`、`--permission-profile`、`-y` / `--yes`、`nola skills`、`nola mcp`、`nola memory`、`nola version` 与 9 种界面语言 `zh` / `zh-hant` / `en` / `de` / `es` / `fr` / `ja` / `ko` / `ru`）。

使用 `nola model --help`、`nola session --help` 或 `nola plugin --help` 可查看对应命令的用法。`nola version --json` 输出版本和构建号。`nola session delete <id> --json` 返回的 `deleted` 表示本次是否实际删除了会话，重复删除返回 `false`。

图像粘贴取决于终端/系统与模型能力。Goal、会话、后台作业和具体操作授权分别有自己的所有者，不靠自然语言关键词选择工具或授予权限。终端原型是布局示意，不添加命令。[验收范围](../../quality/VALIDATION_SCOPE.md)不会用浏览器原型关闭 CLI 的未测交互。


## 网页检索与代理 DNS

`web_search.limit` 决定返回和解析的结果数量。无可用正文证据的候选保留在完整诊断报告中，不提升为可引用的模型来源。

`web_fetch` 在每次连接和重定向前校验网址与解析地址，并固定已校验的公网地址。系统 DNS 若仅返回 `198.18.0.0/15` Fake-IP 别名，会通过固定的 Google Public DNS HTTPS 接口查询真实地址，再执行相同的公网校验；不会直接连接 Fake-IP、内网、回环或元数据地址。该恢复查询仅发送域名，不发送网页路径、查询参数或凭据，并关闭 EDNS 客户端子网信息。设置 `NOLA_WEB_FAKE_IP_RECOVERY=0` 可关闭公共解析恢复；失败提示会指引为代理配置真实 IP 解析或改用已连接浏览器。

## Pi 与 DeepSeek Harness 原生插件

在桌面 Host 上安装官方 Pi 或 DSH，并让 `pi` 或 `dsh` 在 PATH 中可用。DSH 的依赖管理需要其支持的 pnpm。以下示例分别使用本地 Pi 扩展目录和 DSH bundle 目录。

```shell
nola plugin pi install ./my-pi-extension
nola plugin pi list
nola plugin pi pages
nola plugin desktop-pages
nola plugin pi open
nola plugin pi remove ./my-pi-extension

nola plugin dsh install ./my-dsh-bundle
nola plugin dsh list
nola plugin dsh open
nola plugin dsh remove my-dsh-bundle
```

Pi 也可传 `npm:@scope/package@version`，DSH 可传 `@scope/package@version`。DSH 移除时使用包在 `package.json` 中声明的名称。`open` 会进入该运行时的会话，沿用 Nola 为它单独保存的插件配置。安装第三方代码后，插件按当前系统用户权限运行。[接入设计与验收](../../architecture/模块/11-Skill插件与MCP/HARNESS_PLUGINS.md)

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
