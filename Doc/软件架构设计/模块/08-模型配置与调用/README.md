> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# MOD-08：模型配置 / 凭据 / Provider adapter

状态：模块合同与受限实现说明；2026-11-08 核对模型原生档位接线。未逐项复核的旧阶段证据不扩大为当前全量验收。

## 设计、输入输出与生命周期

2026-10-16 细化：实际 PreparedModelRequest 拥有缓存身份；完整目录摘要不能代表本次实际 schema/system 前缀。提示只渲染一次；恢复不得无谓切换工具集；实际变化必须诊断。详见[根因边界](../../总体设计/执行权限与证据边界_20261016.md)。

- 配置解析所有者：共享 Application/Domain 解析器；会话显式覆盖 > 全局，未设置与显式空值严格区分。
- 输入：模型 profile revision、端点协议、会话参数、请求目的、能力声明、credential_ref。
- 输出：冻结的 EffectiveModelInvocation（包括参数来源、预算、工具集合、route identity）。
- Host 安全读取凭据，provider adapter 负责 wire/stream/usage 归一化，不决定工具授权或任务是否完成。
- 首模型请求与维护请求都有独立 attempt/usage；无 usage 是 unknown，不是 0%。
- 错误：配置拒绝、凭据不可用、provider 失败、incomplete 与 transport loss 分开。
- 验证：多端点保存隔离、优先级、协议切换、冻结/重启、重复 usage 去重、各协议缓存计量。

当前单版本准入要求 Host 明确提交 `model_capability`：profile ID 非空、无首尾空白，
revision 为正数。Core 新建、配置更新和 checkpoint 恢复均校验，不再用空 profile
触发推测工具/视觉能力的 resolver。Host 的显式运输协议适配仍可构造 profile；Android
从冻结的模型能力声明与实际 adapter 能力取交集，不能按模型名称或本地来源猜测支持。
测试 Host 同样显式声明，不能修改生产 Default 为全能以维持旧用例。

`model_capability_id/revision` 与 profile 自身身份是不同合同，不能擅自施加镜像等式。历史 CLI provider 猜测问题须按现有配置准入/adapter 单独复核，不能把旧“本批未验证”当作当前事实，也不能未经调用链验证就宣布已消除。当前有效参数入口与逻辑目标类型的区别见[模型配置设计](MODEL_CONFIGURATION_REWRITE.md)。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

模型目录、会话覆盖、冻结调用、凭据引用分离；会话对象不持有凭据明文。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [model.rs](../../../../core/nola-application/src/model.rs) | 连接、账户、catalog、profile、发现/配置命令 |
| [model_profile_resolution.rs](../../../../core/nola-application/src/model_profile_resolution.rs) | profile/会话覆盖到有效调用的解析 |
| [lib.rs](../../../../core/nola-credential-host/src/lib.rs) | 凭据保管与解析的物理 Host 边界 |

有效调用并非一个已完成统一的抽象 DTO：当前代码有 ResolvedRemoteModelProfile/ResolvedModelRuntime、Host 绑定与 PreparedModelRequest。它们的解析和冻结时点须分开记录。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [连接、账户与来源导入](MODEL_CONNECTION_IMPORT.md)
- [GENERATION_OVERRIDE_CONTRACT](GENERATION_OVERRIDE_CONTRACT.md)
- [PROVIDER_INSTRUCTION_PROJECTION](PROVIDER_INSTRUCTION_PROJECTION.md)
- [SHARED_EXECUTION_HOST_CONFIGURATION_20261015](SHARED_EXECUTION_HOST_CONFIGURATION_20261015.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 跨端解析收敛（2026-11-02）
所选 profile/revision 的端点、能力及 reasoning 由 Application 的 resolve_remote_model_profile 查询同一个 resolver；端侧不按全局首 profile 限制其他连接，也不维护 reasoning 白名单。凭据按已验证 connection/account 引用在本机解析，缺少本机凭据明确不可用，禁止借用另一账户的全局 key。

原生能力边界（2026-11-08）：iOS/Android 已投影模型声明的实际 effort，并通过共享 profile resolver 传入调用；不再描述为 iOS 只能执行 automatic。Mac 本机与 Web 同样保留精确 effort，聊天不再并列粗粒度三档。连接 Host 与 budget UI 的差异不能从本机 effort 测试推导，见[跨端配置接线](../13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。Android 手动连接的 secret 使用独立 credential reference，不借用另一账户的全局 key；本次不迁移或改写用户配置。
