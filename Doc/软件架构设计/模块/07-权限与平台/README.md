> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# MOD-07：权限与平台能力

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

本批产品只有 cautious/allow_all（谨慎/全部允许），删除三档定义和旧值兼容。敏感操作按具体资源/操作确认，普通读取不逐 SQL 参数确认；全部允许零应用确认。活动政策通过提交及 ACK 生效，见[根因边界](../../总体设计/执行权限与证据边界_20261016.md)。

- Core 拥有授权策略；Application 保存 request/grant/decision；平台 Host 拥有 OS 授权事实。
- 输入：具体 principal、capability、resource scope、操作、授权模式、OS facts、政策 revision。
- 输出：Allow(receipt) / Ask(request) / Deny(reason)，不是一个全局 approved 布尔值。
- Grant 作用域与寿命明确：once/session/persistent；撤销、模型重试、子 agent 和远端 Host 都要重新核对有效性。
- 自动批准不改变 OS/资源约束。等待授权不重复执行、不消耗模型 retry；点击结果必须绑定 wait_id。
- 验证：跨会话/Host 泄漏、撤权竞态、相同 once 重放、未知资源、自动批准与 OS 拒绝组合。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

PermissionRequest、Grant、Decision 分离；平台权限适配器不能修改 Core 授权策略。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [permission_policy.rs](../../../../core/nola-agent-engine/src/agent/runtime/permission_policy.rs) | 具体调用的政策判断与等待 |
| [permission_source.rs](../../../../core/nola-store/src/permission_source.rs) | 权限来源及提交前沿校验 |
| [platform_catalog_contract_tests.rs](../../../../cli/src/tools/platform_catalog_contract_tests.rs) | Host 实际注册工具与平台目录边界回归 |

平台工具目录、政策判断、持久等待/决定、OS 授权和结果披露是不同边界。已注册 handler 不等于当前 Run 已准入；不得按自然语言关键词修补路由或授予权限。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

### 共享 Host 工具与客户端权限目录的一致性

执行目录的权威来源是 Rust Host 的 `tools::specs()`；Mac 的
`MacCoreToolPolicy` 只为权限挑战提供身份绑定能力凭据，不定义另一个执行器。
它将目录划分为共享文件/进程/编排工具、网络、Mac 日历提醒、托管浏览器、
原生浏览器及桌面操作。分区必须互斥，合集必须恰好覆盖实际注册工具；
删除工具时同步删除凭据入口，不能保留旧名字。新增工具须同时具备 Core
策略元数据，否则不能因 Host 有 handler 就宣称平台已经接通。

`cli/src/tools/platform_catalog_contract_tests.rs` 用实际 Rust 注册结果核对
Mac 分区集合及 Core 元数据，阻止手写目录继续漂移。Mac 原生测试另外核对
身份绑定后工具仍在 enabled 集合；集合检查不能替代真实平台执行验收。
远端开发工具包含后台任务能力，须在凭据中同时声明 background；
缺少该能力会导致已注册的终端续输与子任务控制被策略层过滤。

iOS/Android 原生设备目录不注入桌面 shell 或语言服务器；这些工具在连接
Agent Host 时由该 Host 提供，客户端只呈现授权及结果。Web 同样使用 Host
执行目录。Core 对 terminal_input、language_service_list/query 明确只接受
AgentHost 路由，不能用 background/files 能力把它们声明为手机原生、云端或
浏览器本地处理器；六平台矩阵测试覆盖这些拒绝分支。
平台可用性不等于用户同意，具体进程/文件、会话所有权以及系统
授权仍由执行时策略验证。Mac 远端配置投影仍需进一步以实际 Host manifest
取代客户端推导；本次集合契约不宣称已经解决动态插件目录或远端 OS 差异。

- [PERMISSION_BOUNDARY_CONTRACT](PERMISSION_BOUNDARY_CONTRACT.md)：包含 CLI 工作区外普通访问、模型主动申请敏感读取审批及 2026-10-20 验证范围。
- [PERMISSION_CLOSURE](PERMISSION_CLOSURE.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。
