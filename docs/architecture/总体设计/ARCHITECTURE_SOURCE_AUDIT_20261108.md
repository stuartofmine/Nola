> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 架构源码核对与当前差距

核对日期：2026-11-08；对象是本地当前工作树，包括未提交实现，不把 HEAD 单独当作全部源码身份。本页核对总体依赖、部署、15 个模块入口及明确失效的描述；不是全仓每个分支的形式验证，也不是本次重新执行的真实模型/真机验收。

本次文档工作没有启动用户数据库、恢复向导、provider 配置转入或任何维护操作。源码存在某条路径只证明它仍存在，不等于它符合当前发布兼容约束。

2026-11-10 更正：以下现行部署表不再把保留的旧服务源码当作当前产品组件；本次没有删除源码或重写原始运行证据。

## 核对方式

- `cargo metadata --offline --no-deps --format-version 1` 读取工作区 normal/build 声明依赖，包含 optional/target 边。结果见[依赖图](DEPENDENCIES.md)。声明不等于实际调用或完整第三方链接闭包。
- 从 Rust Host 的字面量 method/path 元组 和 `ApplicationCommand::wire_command_name` 提取[接口索引](../模块/01-领域与协议/INTERFACE_CATALOG.md)。顶层 variant 集合与 tag 映射逐项比对；动态/前置路由另行说明。
- 每个模块 README 指向实际存在的实现和端口；对过时状态及冲突章节直接修正。已实现类型、调用方接线、持久闭环、运行证据分别判断。

## 当前物理部署

| 部署 | 持久业务入口 | 物理执行与制品 | 不能推导的结论 |
| --- | --- | --- | --- |
| CLI / Rust Web | 本地 Application Service，经当前版本协议与数据库身份定位 | `nola` 是产品 CLI/Web；`nola-agent-host` 是独立 Host 入口 | Web 请求返回不等于 Run 终态；CLI 与 Web 同 schema 不保证启动到了同一数据库 |
| macOS | `MacApplicationPort` / `MacApplicationServiceClient`，由 `nola-service` 协调提交 | `MacAgentHostRuntime` 管理 Host 进程，SwiftUI 消费投影 | 不是交互 TUI 驱动，也不是所有 Agent 逻辑都在 Swift 进程内 |
| iOS / Android | `nola_mobile_bindings` 的 embedded Application，复用 bootstrap 和 Store 用例 | bindings 产出 staticlib/cdylib/rlib；平台绑定管理句柄、Host 与生命周期 | 不是每个 ViewModel 一份业务 Core；移动 OS 不保证后台常驻 |
| 连接的执行 Host | 准入绑定的源 Host、Run 与 authority | paired mobile/Web 传输及当前 owner/cursor 恢复 | 本地 replica 不获得执行权或源 OS 授权 |

[Core/facade](../../../core/Cargo.toml) 只产出 rlib；[mobile bindings](../../../core/nola-mobile-bindings/Cargo.toml) 才是最终原生库。bindings 显式 force-link Core 辅助 ABI；打包脚本可将归档重命名为 `libnola_core.a`，文件名不能倒推 crate 所有权。

## 15 个模块的核对结果

| 模块 | 当前事实 | 已纠正的文档问题或保留边界 |
| --- | --- | --- |
| 01 领域/协议 | Domain 值、Service 协议、Host 类型族独立 | 已更新顶层 tag/固定路由索引；错误语义目标不再伪装成已存在的 wire 字段 |
| 02 状态机 | `nola_agent_engine::AgentReducerPort` 拥有候选 reduction/恢复；Kernel 提供纯决策；Core 为 facade | 不再描述为 `core/src/agent` 尚待抽取的第二份状态机 |
| 03 Application | 泛型 `ApplicationRuntime<S>` 使用 Store Port；bootstrap 装配 NolaStore | Service/embedded 是不同部署；查询、mutating 命令、回执与业务终态不同 |
| 04 存储 | `schema_store.rs` 组合多个 DDL，核对 application_id/user_version/schema_epoch；canonical text/journal 在 SQLite 有各自记录 | 删除固定 `~/.nola/nola.db`、唯一 schema.sql、所有长文都在 blobs 的错误断言 |
| 05 执行恢复 | 通用 Driver 使用 committed 端口；Goal 用例协调归属 Application，Driver 保留任务执行适配 | 2026-11-09 已移除 Application→Driver 和 Driver→engine 的生产边，边界门禁覆盖；仍需逐用例验证副作用归属 |
| 06 工具证据 | Application 原始来源提交；Store 已有归档正文 validator/installer 与冷读/分支路径 | 旧“validator 未实现、Apple 必须暂停”的交接状态失效；replica 来源导入不因普通 journal 复制而自动成立 |
| 07 权限 | Core 政策、持久等待/决定、实际资源与 OS 授权各有边界 | handler 注册、目录启用、一次连接批准、单次工具批准不能互相代替 |
| 08 模型 | 连接/账户/catalog/profile 分离；profile/会话解析、Host binding、PreparedModelRequest 各有时点 | 不用逻辑 EffectiveInvocation 名称证明全部调用已统一；原生档位见现行模型合同 |
| 09 上下文 | 原文、逻辑 Context、压缩候选、物理请求热快照和 usage 分离 | 不能把旧超限/预算观察泛化为当前无硬检查；缺计量不补零 |
| 10 Goal/Task/Room | Goal 执行模板、任务租约、Room submission/routing、成员 Context 和移动 child pump 已有源码 | Room 当前仍存在；主控结构化选择也已实现；两者都不能用产品目标反向抹去 |
| 11 Skill/MCP | Skill/MCP 用例与共享 `nola_mcp_host` 已存在，移动本机 HTTP 与桌面 stdio 分开 | 不再把全部 MCP 当作 CLI 私有实现；共享 crate 仍依赖 Core facade，单靠类型存在不能证明原生 child 全覆盖 |
| 12 记忆/数据源/自动化 | 记忆用例、数据源快照与 typed automation spec/execution binding 分离 | 不是只靠不透明 metadata 执行；memory owner 已贯通当前 Recall/Store，平台与真实模型验收范围见下表 |
| 13 客户端 | 只消费版本化投影、提交用户命令；草稿/IME/滚动是展示层 | FFI 制品、当前平台接线与交互见专项合同，不据 UI 或 FFI 返回制造终态 |
| 14 Host/同步 | Rust Host、source publication、replica 和可选 Tailcat 是不同角色 | 不把可选 SDK 或一次连接测试写成全平台传输/后台恢复已验收 |
| 15 观测 | Measurement 是有界非权威诊断；静态门禁、隔离存储、真实执行和真机证据不同 | 历史通过数、一次 EPERM、未运行记录不成为当前全局通过/暂停结论 |

每行的精确源码链接与相关契约见[模块目录](../模块契约.md)，不是新增 15 套运行模型。

## 实现进展与剩余差距

| 差距 | 源码依据 | 当前结论与约束 |
| --- | --- | --- |
| 记忆 owner 接线与验收范围 | [MemoryRecallQuery](../../../core/nola-application/src/memory.rs)、[Domain owner](../../../core/nola-domain/src/memory_owner.rs)、[Store](../../../core/nola-store/src/memory_store.rs) | 已新增必填 record.owner/binding、SQL 排名与 LIMIT 前 owner 过滤、owner 去重及不可变身份；Host 使用显式上下文，移动 profile 搜索走 Recall。原生 workspace/turn 选择 UI 和真实并发 provider 验收仍需独立验证，见记忆合同 |
| 历史 provider 恢复清理 | [bootstrap](../../../core/nola-application-bootstrap/src/lib.rs)、[Mac Service supervisor](../../../NolaMac/Services/MacApplicationServiceClient.swift) | 已删除旧 schema provider 解码/转换、恢复标记、Service 参数与 Mac 普通启动扫描归档。显式归档只保留原文件，新库不导入旧配置；默认不匹配拒绝且保留原库 |
| 错误模型不完全收敛 | [ProtocolFailure](../../../core/nola-protocol/src/lib.rs)、Application/Store Port | wire 只有 code/message/recoverable/details?，部分内部端口仍 String；目标错误语义不能冒充已经类型化，更不能按 recoverable 布尔值盲目重试副作用 |
| 纯分层与实际声明存在耦合 | [Application Goal 协调](../../../core/nola-application/src/goal_coordinator.rs)、[Application manifest](../../../core/nola-application/Cargo.toml)、[MCP manifest](../../../core/nola-mcp-host/Cargo.toml) | 如需进一步收敛，必须按实际用例调整类型所有权、调用者与门禁；文档不能隐去真实边或靠改包名宣称完成 |
| 统一对话目标未等于底层对象删除 | [Room submission](../../../core/nola-application/src/agent_room/submission.rs)、[Room 用例](../../../core/nola-application/src/agent_room.rs) | 当前仍有 Room 身份、显式成员与主控选择路径；它们不是因文档目标存在就已删除的旧入口，也不授权增加新兼容分支 |
| 自动化全平台能力不能统一推定 | [execution spec](../../../core/nola-domain/src/automation_execution.rs)、[occurrence query](../../../core/nola-application/src/automation_occurrence.rs)、各 Host scheduler | Agent/UserTask、发生时间计算、领取执行与 OS 唤醒是不同层；不能从 occurrence 查询通过推导所有平台持续调度或所有 source/script 流程一致 |

差距是对当前代码的记录，不是允许执行数据转换、放宽权限或自动重跑的授权，未完成行不构成已修复代码的声明。现行工作入口继续记录在当前任务（本地工作记录，不随仓库分发）。

## 验证范围

文档核对之后，本轮继续修改 memory owner 和 provider 恢复调用链，并运行相应 Rust/Apple/Android 检查；具体结果以本轮测试记录为准。未读取用户库或验证真实 provider，也不据此宣称所有模块和所有平台全量通过。历史证据只能证明原报告的版本、环境与路径；当前文档不重新赋予那些报告新的代码身份。
