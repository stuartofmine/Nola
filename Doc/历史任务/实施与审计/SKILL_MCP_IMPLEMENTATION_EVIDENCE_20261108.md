# 架构历史实施记录

2026-11-08 从现行架构中分离。下文保留原日期、失败/通过与暂停范围，不构成当前实施状态、操作限制或兼容要求；相对链接按归档位置调整。

## 当前差距与验收

2026-11-04 接线整改（当前代码，不代表全平台动态验收）：

- Web 自定义 Skill 使用 Core `ListEditors` 返回的 revision 作为编辑基线，通过一次 `ApplyChanges` 提交 `CreateUser` / `ConfigureUser` / `DeleteUserChecked`。过期编辑拒绝，不覆盖新版本；其他编辑者新增且不在基线内的 Skill 不被删除。`ConfigureUser` 在同一事务应用明确提交的 enabled 状态。
- OpenCode MCP `command` argv 数组与 `environment` 在 Host 的外部配置入口解析；内部仍使用既有命令/参数表示。HTTP 来源的 `url` 在同一绑定入口解析为 endpoint；连接仍需原有批准，不因导入自动连接。
- Mac 本地 MCP 的 server ID 来自来源资源身份摘要，同名不同来源不再共享连接身份；显示名不变。
- 首轮真实隔离 GPT 验证已观察 Skill 加载、工具发现和 `mcp_connect` 请求。Cautious 策略要求新连接显式批准；无交互批准通道时停在 AwaitingPermission。这不是 MCP 工具执行成功的证据，不得通过改为 AllowAll 宣称通过。

后续接线（2026-11-04）：

- Web 内置 Skill 的正文、启用状态与 revision 也来自 Core。`ConfigureBuiltIn` 在既有编辑事务内组合正文选择和启用操作，Web 不再持久化第二份 override。旧 settings 中非空 override 不被当作运行配置或自动转换：页面提供明确审阅入口，先原样保存在 `web.skill-settings.unapplied.<digest>`，再经用户确认及设置 revision CAS 选择 Core 配置。原有其他字段不变；备份始终可在页面查看，用户可手动重新应用需要的编辑。选择后立即按现有生命周期生成 Skill 运行资源。
- Web 提供新增本地 stdio MCP，默认未启用；启动配置只编辑 command/args/cwd，既有环境变量不进入编辑区。保存不建立连接，也不构成授权。
- 普通 CLI Run / Room 从相同 Application MCP 记录加载配置；Native stdin 入口保留调用方的明确选择，不因空目录而扩大配置来源。
- 子代理保留配置来源、重新绑定自身 Host/session，不继承父连接的 live catalog/generation。无额外限制的 profile 使用原有渐进发现，不再被启动时静态工具名单阻断；显式工具名单、只读要求和 Core 每次权限检查保持。
- MCP 工具展示 schema 会附加描述。安装和实际执行的目录围栏必须使用同一生成规则，不能拿展示 schema 与未经附加描述的原 schema 比较；实际 schema 变化仍须正式刷新。

动态边界：修后隔离 GPT 通过 Skill 加载、发现、一次批准连接、独立批准工具调用，MCP 服务收到一次只读 lookup，封存答案与返回值/证明码一致；具体请求、成功/失败与用量见本地验收记录。子代理实际模型调用、全平台原生页面及新 stdio 的浏览器实际操作不因 Core 回归通过而视作已验收。托管 Skill 资源的首次初始化/用户明确保存仍沿现有生命周期处理，不把外部显式目录当可覆盖的缓存。


最近此前实施与验证见[阶段归档](../2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../../软件架构设计/模块/15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 2026-11-04 父子与 Web 实测补充

- 共享 Host 工厂将 `SessionPermissionScope.owner_session_id` 绑定到真实会话；授权缓存 namespace 仍可用 `host:task:<id>`。二者不能混为 MCP / 浏览器 / 终端资源所有权。MCP 的严格 Host/session/config/generation 校验未移除。
- 内置 Skill 的本地用户 selection/override 需要恢复本地归属 envelope 后才能走 ACL 投影；仅对已知嵌入包生成的本地记录这样处理，不能给任意外部资源补本地所有权。修前 Web 可见内置目录为 0，修后为 24。
- 真实 GPT 父任务委派给真实同构建子进程；子会话独立加载 Skill、发现和连接 MCP。父任务通过既有 Task permission API 分别一次批准精确连接与只读调用，foreign owner 无法读取其授权提示。服务收到一次 lookup，父子封存答案都与返回值/证明码一致。此前两处身份混用失败轨迹保留，不计为通过。
- 内置浏览器实际执行了旧设置审阅/保留备份/选择 Core、24 项内置目录显示、同时修改正文与停用后保存重开，以及 stdio 新增/重开/启用。保存和启用未自动启动 stdio 进程。Web 备份在后续 Skill 编辑后仍可查看，其他设置保持。
- 此处不是全平台原生验收：原生窗口清单工具仍返回自动解锁错误，未确认实际锁屏状态且未重试解锁。此次新 stdio 仅验证 Web 配置交互及不自动执行；真实父子 MCP 调用使用自建只读 HTTP 样例。无提供方登录、真实配置修改或推送。

