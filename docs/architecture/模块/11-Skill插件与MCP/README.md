> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-11：Skill / MCP / Catalog

简体中文 · [English](README.en.md)

安装前先选资源和运行位置。GitHub Skill 固定仓库提交，MCP 连接按 Host 提供的传输建立。桌面插件的页面入口来自安装包声明；需要 Pi 或 DSH 的界面交给对应运行时。手机本机和配对宿主分别维护能力。

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## Pi 和 DeepSeek Harness

桌面应用从本机已安装包发现页面，将每个可用页面加入主侧栏。用户可在同一窗口切换插件工作页和对话。Pi 自绘组件和 DSH 客户端插槽由对应运行时加载。页面声明、安装位置与配对 Host 的边界见[插件接入设计](HARNESS_PLUGINS.md)。

## 从对话安装公开 GitHub Skill

用户可以提供名称、仓库、子目录或 `SKILL.md` 链接。模型根据已公布的工具搜索和选择，仓库中有多个符合用途的候选时，可以先让用户选。一次安装固定到一个 Git 提交，同时下载所选目录和 `manifest.yaml` 的 `always_load` 引用文件。

| 使用方式 | 包与启用状态归属 |
| --- | --- |
| iOS、Android 本机对话 | 手机应用私有目录与本机 Skill 设置。新对话按本机启用状态加载。 |
| 手机连接 Host 的对话 | 安装到所选 Host 后，包与开关保存在该 Host。设备工具与 Host 工具使用不同的标识和权限。 |
| macOS 原生对话 | Mac 应用的数据库保存 Skill 记录，内置 Agent Host 在新会话启动时加载已启用的包。 |
| Web、CLI、Windows Tauri | 运行该会话的 Rust Host 负责下载、登记和启用。Windows 的桌面壳启动打包的本地 Web Host。 |

共享包实现位于 [nola-skill-host](../../../../crates/nola-modules)。搜索为只读操作，安装需要持久写入许可。移动端通过应用自己的编辑接口登记准备好的包，读取引用文件时校验启用状态、资源权限和包边界。下载脚本与执行脚本分别授权。GitHub 限制公共 API 请求时，仓库和目录链接可以通过公开归档解析到具体提交。归档索引下载上限为 64 MiB，选中的 Skill 连同声明的依赖文件上限为 32 MiB、1024 个文件。

## 设计、输入输出与生命周期

- Core 拥有能力选择和激活语义；Host 加载资源/执行 MCP transport；Store 保存配置、版本及授权记录。
- 输入：模型选定的 schema 调用、catalog revision、已授权资源。输出：能力描述/结果/恢复建议。
- 外部描述和 Skill 内容不授予权限；catalog 缓存失效不能静默换用不同 schema。
- 检索工具名不依赖用户语言关键词；未知工具可恢复但不直接执行近似匹配能力。
- 验证：版本失配、断连、未知工具、多语言自然追问、激活与普通工具批次屏障。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

Catalog、Activation、Transport 分离；插件适配器只通过版本化能力端口接入。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [skill.rs](../../../../crates/nola-modules/src/skills.rs) | 内置/用户 Skill 的版本化编辑用例 |
| [mcp.rs](../../../../crates/nola-mcp/src/lib.rs) | MCP 持久目录与配置命令 |
| [lib.rs](../../../../crates/nola-mcp/src/lib.rs) | 跨 CLI/移动的连接、HTTP/stdio 与移动适配模块 |

共享 nola_mcp_host 已存在，且仍依赖 Core facade；移动本机支持范围与桌面 stdio 分开。保存/启用配置不等于连接授权，连接授权不等于每次工具调用授权。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [skill-runtime-architecture](skill-runtime-architecture.md)
- [SCALE_AND_PLUGIN_RUNTIME_20261015](SCALE_AND_PLUGIN_RUNTIME_20261015.md)

## 当前配置与执行边界

Skill 目录、正文、启用状态和 revision 由 Application 的 Skill 用例与资源生命周期拥有；MCP 配置由 McpCommand/Repository 拥有。客户端编辑基于当前 revision，不用整包陈旧设置删除另一编辑者新增的项目。旧用户设置审阅、备份、重新应用等阶段记录不是现行兼容要求；不据此保留历史配置转换。

`nola_mcp_host` 已提供连接身份、保留连接、发现 catalog、取消及 HTTP/stdio 模块，CLI `mcp_host.rs` 复用共享实现，移动 bindings 使用 mobile adapter。它仍声明依赖 nola_core，不能描述成已彻底抽离的纯 Domain 组件。

目录、启用配置、连接授权与单次调用准入分别校验；每次物理调用仍绑定原 Host/session/config/generation 与已提交执行权。主控与子任务不能共享父连接 live handle。原生 child 的真实处理器集合和平台 OS 能力须独立核验，不从 CLI 一次父子实测推定所有平台支持。

此前日期化真实模型、Web 编辑及失败轨迹保存在历史记录（本地工作记录，不随仓库分发）；不重新标记为当前全平台验收。

## 移动端本机与远程 Host 的当前能力边界

- CLI/Web 与 iOS/Android 复用 `core/nola-mcp-host` 的连接身份、保留连接、HTTP 协议、发现目录、工具描述与取消语义。CLI 的原模块只重导出共享实现；进程传输仍由具备进程执行能力的桌面 Host 使用。
- 移动端本机支持已保存的 HTTP(S) MCP。读取配置不会建立连接；模型通过 `mcp_connect` 请求一次具体连接，经 Core 审批后初始化并发现工具。每次工具调用仍需独立 Core 准入和审批，结果将返回当前模型。连接授权不等于工具调用授权，也不产生设备读取或持续结果披露凭据。
- Application 在物理 I/O 前核对已提交的 ExecuteTool、Run、owner、delivery、worker 和 lease，并从当前已提交 Core 状态校验动态激活的描述；Host 再核对当前启用配置、精确 session/Host 与配置 digest。客户端不自行创建工具路由或决定授权。
- 共享目录投影保留原始会话 allow/deny、风险和生命周期事实；不从“当前启用工具集合”反推权限。Android 的托管 MCP 配置先经过现有资源读取权限检查；不可读记录不安装到本机 Host。
- 禁用、删除或修改配置后，旧连接不能被用于新的调用；未知执行结果不会自动重发。取消发送 MCP cancellation 通知，但发送通知本身不证明远端副作用已停止。
- iOS 不启动 MCP 子进程。本轮 Android 本机同样只开放 HTTP，不开放任意程序启动。stdio 配置须在具备相应能力的已连接 Host 上运行；移动端不会将本机配置或授权静默复制到远程 Host。
- 原生 Skill 保持独立的 `skill_invoke` 与上下文快照语义；Skill 成功不证明 MCP 调用。跨 child session 不能复用父会话连接引用；当前原生子任务的固定处理器交集尚未接入 MCP，本轮不声明子任务 MCP 可用；后续须按 child 自身冻结目录与 owner 独立准入，不能借用父连接。
