# Pi 和 DeepSeek Harness 插件

简体中文 · [English](HARNESS_PLUGINS.en.md)

用户可以在同一台桌面 Host 上分别管理 Pi 扩展和 DeepSeek Harness 插件。Pi 的工具、命令、事件与界面由 Pi 加载。DeepSeek Harness 的 Cordis 服务和补丁由 DSH 加载。独立的 `SKILL.md` 沿用现有 Skill 安装入口。一个仓库同时包含多种资源时，安装请求要标明选中的资源和运行位置。

## 桌面 Host 已实现的入口

`nola plugin pi|dsh` 调用 PATH 上已安装的官方运行时，透传原生输出。包名、Git 来源和本地包目录使用所选运行时的格式。管理命令在专用工作目录执行，显式相对目录按调用者当前目录解析。`open` 在 Nola 选中的项目打开该运行时的交互会话。

| 用户操作 | Pi | DeepSeek Harness |
| --- | --- | --- |
| 安装 | `pi install <source>` | `dsh plugin --profile tui add <source>` |
| 查看清单 | `pi list` | `dsh plugin --profile tui list --depth 0` |
| 使用已启用插件 | 启动 Pi 会话 | 启动 DSH 的 `tui` profile |
| 移除 | `pi remove <source>` | `dsh plugin --profile tui remove <package-name>` |

运行时状态放在 Nola 数据库旁的 `<database-stem>.plugins/harness/<runtime>/`。Pi 使用专用的 `PI_CODING_AGENT_DIR`。DSH 使用专用的 `DSH_HOME` 和 `DSH_AGENTS_HOME`。运行时自己管理安装记录、依赖和版本检查。DSH 的清单包括普通依赖；有可用 bundle 声明的依赖才能自动挂入 profile。运行时的失败退出会反馈给 CLI，安装失败后可以重新查看当前清单。

用户直接执行安装和打开命令时，插件代码可能以当前系统用户身份运行。单独的配置目录用于划分状态，执行权限仍来自操作系统用户。子进程只接收启动、终端、临时目录和网络所需的环境变量。模型账户在对应运行时里配置。现有 Nola 原生插件管理、协议和数据库格式沿用已发布的接口。

## 接入 Nola 主对话与客户端的后续设计

1. **从对话选择目标。** Host 报告运行时、版本、可用操作和配对身份。安装审阅显示插件来源、版本、将运行第三方代码的 Host，以及使用的 profile。模型按工具 schema 提出具体请求，Core 对该请求授权。取消审阅保留原清单。手机同时展示本机支持的标准 Skill 和有能力的配对 Host。
2. **区分谁在运行。** 需要 Pi 命令、TUI 或 DSH UI 的插件在原生会话中使用。对外部 Agent 任务，主 Agent 可以把任务发给对应运行时，再把结果挂回原会话。普通对话不直接启动任意第三方代码。
3. **把可调用的能力接回主会话。** 优先复用已有 MCP 服务接入路径。需要原生桥接的插件须由适配器从已加载实例读取工具名称、输入 schema 和生命周期。Core 将描述绑定到 Host、运行时、安装版本与会话后再公布。调用时重新核对实例、参数与授权；交互式输入、取消和未知结果使用现有 Host 协议。依赖专属 UI 的能力保留在原生界面。
4. **跨设备管理。** 桌面图形客户端和手机向同一 Host 请求清单及安装操作。界面分别显示已下载、已启用和当前会话可调用。断开连接时保留操作位置和最后一次可确认的状态；重新连接后先查询，避免重复安装或调用。手机独立模式继续使用已有 Skill 和 HTTP MCP 能力，执行 Pi / Cordis 原生插件需要具备对应运行时的 Host。

后续接线的验收包括同名工具的来源选择、拒绝安装、启用后调用、移除或改版后旧调用失效、跨会话隔离、手机断连恢复。需要在真实模型会话和配对设备上逐项验证。

## 如何复测首阶段

在 macOS 或 Linux 上将 `nola`、官方 `pi`、`dsh` 及 DSH 所需的包管理器装好，通过以下入口复测。脚本使用临时用户目录、数据库路径和两个本地插件包。它实际打开终端会话，检查扩展入口执行的标记，然后移除插件并重新读取清单。

```sh
./scripts/test_rust.sh nola_cli harness_plugin
python3 scripts/test_harness_plugins.py --nola target/debug/nola
```

macOS 实测使用 Pi 1.5.2 和 DSH 0.2.2-rc.5。两个本地插件都在新的原生会话中执行了加载入口。测试也覆盖运行时缺失、安装来源不存在时返回失败，以及失败后原插件依然可用。另外从公开 npm registry 分别安装、列出并移除了 `pi-statusline@0.0.2` 和 `@deepseek-ai/dsh-experimental-badge-skill-bundle@0.2.2-rc.5`。公开包测试覆盖包管理，原生加载由本地插件用例验证。

[Pi 包文档](https://docs.pi.dev/latest/packages) · [DSH 官方 CLI](https://github.com/deepseek-ai/deepseek-harness/tree/master/apps/cli) · [Nola CLI 用法](../../../platforms/cli/README.md)

## 全平台管理界面

[插件工作区交互设计](../../../design/shared/PLUGIN_WORKSPACE.md)定义从安装到日常使用的流程。

同一个插件管理页属于一个明确的 Host。进入后汇总 Pi、DSH 终端及 Web 的已安装包。添加时选择运行时和目标配置，移除时沿用条目所属配置。清单按包显示，空清单、运行时缺失、读取失败各有独立反馈。原生工具与命令在对应运行时会话里使用。

| 入口 | 交互 |
| --- | --- |
| Web、Windows Tauri | 从能力管理进入插件页，使用当前 Web Host。Windows 复用本地 Host 的管理界面。 |
| macOS | 扩展中心管理本机安装。主侧栏直接进入插件页面。远程管理使用已有的配对交接。 |
| iOS、Android | 从能力库进入；使用已配对的桌面 Host，先显示设备位置。缺少配对时，引导到已有连接入口。 |
| CLI | 在终端选择对应运行时，通过 install、list、open、remove 管理；pages 列出当前 profile 中已安装包的已登记页面及地址；保留插件的原生终端交互。 |

添加插件先填写 npm 包名及可选版本，再审阅来源、运行时和目标 Host。返回编辑保留输入。移除前确认包名；提交时锁定当前操作。操作后重新读取清单；断连后提供刷新，用户核对状态再决定是否重试。页面显示在 Host 的终端打开对应会话的命令。手机按单列排列，主操作和切换项至少 44 点高；沿用各平台主题与键盘焦点样式。

## 桌面插件与主窗口

安装一个带有「任务看板」页面的包后，用户在 Nola 左侧栏看到「任务看板」。点击它，主内容区切换到看板，侧栏保留。点击原对话可以继续聊天。以后启动 Nola 仍从已安装包恢复入口。移除包会撤回它贡献的全部页面。没有页面的扩展只贡献自身能力。扩展中心负责安装和移除，日常使用从主导航进入。

Mac 使用本机安装清单及内置 CLI。Windows 使用随应用运行的本机 Host，网页客户端使用当前连接的 Host。仅加载当前运行时、profile 与已安装包共同确定的页面。每个页面都有独立标识，两个插件的页面即便同名也分别可达。授权和安装位置沿用原有运行时。

### 插件作者如何提供页面

随包发布一个自包含 HTML 文件，在 `package.json` 的 `nola.pages` 中声明稳定的页面 ID、用户可读的标题和相对入口。举例：

```json
{"name":"example-task-board","nola":{"schema_version":1,"pages":[{"id":"board","title":"任务看板","entry":"ui/board.html"}]}}
```

Nola 从已安装包发现声明。点击时才读取页面内容，使用与主应用隔离的 Web 视图展示，可执行页面内自己的交互。打开一个页面无需额外启动 HTTP 服务。这个接口适合看板、报表和自带前端的编辑器；需要宿主权限、持久状态或远程服务的页面要有对应的受控接口。Pi 自绘终端组件继续由 Pi 渲染，DSH 专属客户端插槽需要相应的运行环境。单独的终端界面或源码仓库地址不能自动转换为桌面页面。

### 已部署的远程服务

首版允许 Host 管理者登记已部署网页。每个运行时在 Nola 数据库旁的 `<database-stem>.plugins/harness/<pi|dsh>/pages.json` 单独配置。文件可省略；只有清单中实际存在且来源字段匹配的包显示入口。登记一个链接不启动 Web 服务，也不给页面注入 Nola 的会话、配对凭据、原生桥或会话上下文。界面显示目的站点，用户点击后才导航。远程手机不会收到指向它自身回环地址的入口。专属 UI 代码在原生运行时或独立站点执行。

```json
{
  "version": 1,
  "pages": [
    {"package": "@example/task-board", "title": "任务看板", "url": "https://board.example.org/tasks"}
  ]
}
```

Pi 的 package 填清单中的完整来源，DSH 填依赖名。URL 使用独立站点的 HTTPS；同机调试可用独立回环主机名的 HTTP。Host 实例报告动态页面、为单次启动签发短期入口和状态回传属于下一阶段。登记项只证明 Host 提供了入口；可用性、登录和页面内权限由目标服务判定。

添加 DSH 插件时可选择「终端 / Web」两种使用位置。安装和移除只影响选中的 profile，已有终端安装保留原来的位置。Web profile 使用 `nola plugin dsh --profile web open` 在 Host 启动；插件管理页不会随打开而启动服务。Web profile 的独立页面使用同目录下的 `web-pages.json` 登记。

## 验证独立页面

在仓库根目录构建 CLI 与 Web 静态资源，准备好 Pi、DSH 及项目的 Browser Use 测试环境后，运行以下验收。

```sh
node apps/web/playwright_runtime/harness_plugins_ui.cjs
```

验收使用临时 Host，实际从 npm 安装包，在桌面和手机宽度下检查取消安装、移除、切换 profile。独立看板由本地测试服务提供，验证登记链接的打开、Cookie 与来源地址隔离、关闭后返回清单，以及卸载后入口消失。桌面流程还会从临时 npm registry 安装带页面声明的测试插件，实际点击侧栏、操作页面、重新加载并卸载。这个测试包的 HTML 随插件发布，运行时不依赖看板服务器。原型的「运行时插件」场景覆盖六个平台；手机配对引导和平台浏览器交接需分别验证。
