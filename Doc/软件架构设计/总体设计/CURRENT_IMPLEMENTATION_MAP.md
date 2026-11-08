# 当前实现与模块映射

核对：2026-11-08，本地工作树。覆盖总体 manifest 图、15 个模块的实现/端口入口和明确陈旧的状态描述，不声称逐行验证全仓库或重新运行所有验收。原设计目标、当前调用接线和历史证据分别维护；实际差距见[源码核对](ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

| 模块 | 当前源码入口 | 实际边界与限制 |
| --- | --- | --- |
| 01-领域与协议 | [lib.rs](../../../core/nola-domain/src/lib.rs) / [lib.rs](../../../core/nola-protocol/src/lib.rs) / [lib.rs](../../../core/nola-host-protocol/src/lib.rs) | 当前 Service wire 的错误字段与目标错误语义仍有差距；不能把逻辑 DTO 表当成已存在的字段。Service、C ABI、Store schema/epoch、Core cursor 与配对协议分别版本化。 |
| 02-Core与状态机 | [reducer_port.rs](../../../core/nola-agent-engine/src/reducer_port.rs) / [submission.rs](../../../core/nola-agent-engine/src/agent/runtime/submission.rs) / [lib.rs](../../../core/src/lib.rs) | 状态机实现已在 agent engine，不能再把 core/src/agent 说成待抽取的另一份实现。facade 仍有纯辅助函数，但最终原生产物属于 mobile_bindings；整个包装 crate 的存在不证明所有路径无 I/O。 |
| 03-Application与事务 | [application_command.rs](../../../core/nola-application/src/application_command.rs) / [lib.rs](../../../core/nola-application/src/lib.rs) / [runtime_owner.rs](../../../core/nola-application-bootstrap/src/runtime_owner.rs) | Service 是本地进程接入，embedded 是移动部署；两者复用 Application 用例。bootstrap 负责具体 Store 装配，不是可绕过命令边界的客户端 SDK。查询不应为读取制造业务回执。 |
| 04-存储与索引 | [schema_store.rs](../../../core/nola-store/src/schema_store.rs) / [canonical_text_store.rs](../../../core/nola-store/src/canonical_text_store.rs) / [assistant_journal.rs](../../../core/nola-store-port/src/assistant_journal.rs) | 同 schema 不等于所有平台共享同一个文件路径；大文本已含 SQLite canonical 对象、分块、stream head/manifest，不能统称 ~/.nola/blobs。schema.sql 是组合 DDL 的一个入口，不是唯一文件。 |
| 05-执行与恢复 | [lib.rs](../../../core/nola-runtime-driver/src/lib.rs) / [task_execution_authority.rs](../../../core/nola-runtime-driver/src/task_execution_authority.rs) / [native_host_entry.rs](../../../cli/src/native_host_entry.rs) | 通用 Driver 循环与同 crate 的 Goal/任务适配要分开描述；当前 Goal 模块仍依赖 engine。进程内 lease handle 只是对已提交授权的引用，重启不能据此恢复执行权。 |
| 06-工具与证据 | [tool_source.rs](../../../core/nola-application/src/tool_source.rs) / [tool_source.rs](../../../core/nola-store/src/tool_source.rs) / [execution_evidence.rs](../../../core/nola-domain/src/execution_evidence.rs) | 归档 validator/installer 和当前来源端口已有实现，旧“未实现、暂停 Apple 编译”不是本次工作限制；是否可跨 replica 导入工具来源须读当前 installer，不能从普通 journal 复制推断。 |
| 07-权限与平台 | [permission_policy.rs](../../../core/nola-agent-engine/src/agent/runtime/permission_policy.rs) / [permission_source.rs](../../../core/nola-store/src/permission_source.rs) / [platform_catalog_contract_tests.rs](../../../cli/src/tools/platform_catalog_contract_tests.rs) | 平台工具目录、政策判断、持久等待/决定、OS 授权和结果披露是不同边界。已注册 handler 不等于当前 Run 已准入；不得按自然语言关键词修补路由或授予权限。 |
| 08-模型配置与调用 | [model.rs](../../../core/nola-application/src/model.rs) / [model_profile_resolution.rs](../../../core/nola-application/src/model_profile_resolution.rs) / [lib.rs](../../../core/nola-credential-host/src/lib.rs) | 有效调用并非一个已完成统一的抽象 DTO：当前代码有 ResolvedRemoteModelProfile/ResolvedModelRuntime、Host 绑定与 PreparedModelRequest。它们的解析和冻结时点须分开记录。 |
| 09-上下文与压缩 | [context.rs](../../../core/nola-application/src/context.rs) / [compaction_driver.rs](../../../core/nola-agent-engine/src/agent/runtime/compaction_driver.rs) / [request_snapshot.rs](../../../cli/src/llm_parts/request_snapshot.rs) | canonical 原文、逻辑工作集、压缩摘要、物理请求快照和计量不能合并为同一历史。热路径资格来自实际请求见证；缓存命中由实际 usage 证明，未知不能显示为零。 |
| 10-Goal任务与子代理 | [goal_execution.rs](../../../core/nola-application/src/goal_execution.rs) / [submission.rs](../../../core/nola-application/src/agent_room/submission.rs) / [child_pump.rs](../../../core/nola-mobile-bindings/src/child_pump.rs) | 当前源码仍有 AgentRoom、Room submission/dispatch 与独立成员 Context；统一 Conversation 是产品方向，不能文档化为底层 Room 已删除。主控结构化路由函数已有实现，也不能再一概标成“未实现”。 |
| 11-Skill插件与MCP | [skill.rs](../../../core/nola-application/src/skill.rs) / [mcp.rs](../../../core/nola-application/src/mcp.rs) / [lib.rs](../../../core/nola-mcp-host/src/lib.rs) | 共享 nola_mcp_host 已存在，且仍依赖 Core facade；移动本机支持范围与桌面 stdio 分开。保存/启用配置不等于连接授权，连接授权不等于每次工具调用授权。 |
| 12-记忆数据源与自动化 | [memory.rs](../../../core/nola-application/src/memory.rs) / [data_source.rs](../../../core/nola-application/src/data_source.rs) / [automation_execution.rs](../../../core/nola-application/src/automation_execution.rs) | AutomationDefinition 已有 typed execution_spec（agent/user_task）；MemoryRecord.owner、RecallQuery.binding 与 Store owner 过滤/去重已接线，移动端 profile 搜索复用 Recall。原生 workspace/turn 选择界面与真实并发模型验收不由这些接口自动推定。 |
| 13-客户端投影 | [lib.rs](../../../core/nola-mobile-bindings/src/lib.rs) / [projection.rs](../../../core/nola-mobile-bindings/src/projection.rs) / [RemoteSubmissionIdentity.swift](../../../Shared/Protocol/RemoteSubmissionIdentity.swift) | 原生静态/动态库由 mobile_bindings 产出，再保留 Core 辅助符号。Swift/Kotlin 的观察、草稿、IME 与焦点不是第二个业务 Runtime；手机导航不改变已准入 Run。 |
| 14-远程Host与同步 | [routes.rs](../../../cli/src/web_parts/routes.rs) / [index.ts](../../../backend/worker/src/index.ts) / [README.md](../../../transport/tailcat/README.md) | Rust 执行 Host、Worker/D1、远端镜像与可选 Tailcat 是不同角色。可选隧道替换承载，不取代配对/登录/CSRF/HMAC、源 Run 权威或恢复游标。 |
| 15-观测与验收 | [lib.rs](../../../core/nola-measurement/src/lib.rs) / [dependency_boundary.rs](../../../cli/tests/dependency_boundary.rs) / [test_mac_layers.sh](../../../scripts/test_mac_layers.sh) | 文档中的历史通过数、一次沙箱拒绝或“尚未运行”都仅属于原快照；当前测试范围以实际命令/报告为准。依赖图、静态检查和组件测试不能推出业务恢复或真机通过。 |

## 部署、制品与版本

桌面 Service、Mac agent Host、移动 embedded Application、Rust Web 和 Worker/D1 的物理部署见[分层](CORE_HOST_LAYERING.md)。同协议/同 schema 不等于同一数据库文件；Store 路径由平台和显式配置决定，不能自动扫描或合并用户库。

`nola_core` 当前只产出 rlib；原生最终产物由 `nola_mobile_bindings` 打包，并显式保留 Core 辅助符号。Core reducer、Application 提交、Store 事务、Driver/Host 物理执行和客户端投影是不同角色。

| 独立版本/身份 | 当前唯一声明来源 |
| --- | --- |
| Service 协议与框架上限 | [nola-protocol](../../../core/nola-protocol/src/lib.rs) |
| Application C ABI | [mobile bindings](../../../core/nola-mobile-bindings/src/lib.rs) |
| Store schema/epoch | [Store constants](../../../core/nola-store/src/lib.rs) 与 [gate](../../../core/nola-store/src/schema_store.rs) |
| checkpoint/reducer/trace | [agent engine](../../../core/nola-agent-engine/src/lib.rs) 与相应格式类型 |
| 配对/源提交/journal/终态 seal | 各自 Domain/Protocol/Host 类型，不能复用 Service 数字或 UI 标签代替 |

## 如何核对

- [依赖图](DEPENDENCIES.md)来自 Cargo metadata 的 normal/build 声明边，包含 optional/target，不是动态调用图。
- [接口索引](../模块/01-领域与协议/INTERFACE_CATALOG.md)区分固定元组、动态/前置路由、Worker dispatch 与 command tag，不把路径存在当作授权或验收。
- [模块契约](../模块契约.md)区分当前 wire 字段和目标错误语义；内部 String 错误及其余未完成项如实列为差距；owner 接线与删除旧 provider 恢复的进展见源码核对。
- 历史暂停/通过记录只约束当时范围，不成为当前自动禁止构建或全量通过结论。当前任务见[任务入口](../../当前任务/README.md)。

本轮没有打开用户数据库、执行恢复/转换或修改运行代码。静态源码与文档一致不等于真实模型、整窗交互、真机后台与跨设备恢复已经验收。
