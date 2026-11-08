> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Rust 生产依赖与边界门禁

核对：2026-11-08，依据当前 `cargo metadata --offline --no-deps --format-version 1`。下表是 normal/build 声明依赖，包含 optional/target 条件边，不包含 dev-dependency；不是运行调用图、实际链接闭包或安全证明。

## 生产声明图

| 实际包名 | 直接工作区生产声明依赖 |
| --- | --- |
| `nola-code-memory` | 无 |
| `nola_agent_engine` | `nola_domain`、`nola_host_protocol`、`nola_kernel` |
| `nola_application` | `nola_agent_engine`、`nola_domain`、`nola_host_protocol`、`nola_kernel`、`nola_measurement`、`nola_protocol`、`nola_runtime_driver`、`nola_store_port` |
| `nola_application_bootstrap` | `nola_application`、`nola_store` |
| `nola_cli` | `nola-code-memory`、`nola_application`、`nola_application_bootstrap`、`nola_code_mode_host`、`nola_core`、`nola_credential_host`、`nola_domain`、`nola_host_protocol`、`nola_mcp_host`、`nola_measurement`、`nola_plugin_host`、`nola_protocol`、`nola_runtime_driver`、`nola_runtime_supervisor`、`nola_service`、`nola_service_client`、`nola_store_port`、`nola_update_host` |
| `nola_code_mode_host` | `nola_domain` |
| `nola_core` | `nola_agent_engine`、`nola_domain`、`nola_host_protocol`、`nola_kernel` |
| `nola_credential_host` | 无 |
| `nola_domain` | 无 |
| `nola_host_protocol` | `nola_domain` |
| `nola_kernel` | `nola_domain` |
| `nola_mcp_host` | `nola_core` |
| `nola_measurement` | 无 |
| `nola_mobile_bindings` | `nola_application`、`nola_application_bootstrap`、`nola_core`、`nola_domain`、`nola_host_protocol`、`nola_kernel`、`nola_mcp_host`、`nola_model_runtime_host`、`nola_protocol`、`nola_runtime_driver`、`nola_update_host` |
| `nola_model_runtime_host` | 无 |
| `nola_plugin_host` | `nola_domain` |
| `nola_protocol` | `nola_domain` |
| `nola_runtime_driver` | `nola_agent_engine`、`nola_domain`、`nola_host_protocol` |
| `nola_runtime_supervisor` | `nola_domain`、`nola_protocol`、`nola_service_client` |
| `nola_service` | `nola_application`、`nola_application_bootstrap`、`nola_domain`、`nola_protocol` |
| `nola_service_client` | `nola_protocol` |
| `nola_store` | `nola_domain`、`nola_kernel`、`nola_measurement`、`nola_store_port` |
| `nola_store_port` | `nola_domain` |

## 所有权规则与待删除的实现耦合

- Domain 值类型、Kernel 决策不依赖应用/存储/Host。当前纯层外部依赖只允许已审阅的序列化、
  URL 值与摘要库；新增网络、数据库或执行库必须失败，不能因为改了依赖别名而绕过。
- Protocol、Host Protocol、Store Port 只引用 Domain；接口不能反向依赖具体实现。
- Store 可使用 Domain/Kernel/Store Port 进行持久化规则校验，不依赖 Application，避免双向所有权。
- Measurement 是非权威诊断接收端，不依赖任何业务模块；Store/CLI 可以发送有界测量记录，
  但诊断失败不得改变业务提交结果。Domain/Kernel 不依赖诊断库；测量不能成为决策输入。
- Application 依赖 Store Port 而非具体 SQLite；测试可以使用真 Store 验证事务。
- Bootstrap 是具体实现装配点，不能把业务决定迁入其中以绕过 Application 规则。
- 当前工作区没有 `legacy_agent_adapter` 包。Application直接依赖 `nola_agent_engine`；Core facade也依赖engine。该实际边不能被旧依赖图掩盖，也不表示engine类型已全部转为纯Domain端口。
- CLI 和 mobile_bindings 仍直接引用 Core，需逐用例收敛；不能以依赖图无环作为“只有 Application 写入”的证明。

当前切片将 `EventCursor`、`CoreEventEnvelope<E>` 与持久来源字段 `CoreEventSource`
统一归属 Domain。Core 负责生成身份，Application 与初始准入序列化同一来源结构，
Store 的输入、要求、执行证据、等待与决定索引共同解析该结构，拒绝额外身份字段和替代游标形状。
这不代替各索引的 Run/Turn 所有权、已提交水位和原始事件核验；纯值结构也不自行授予来源可信性。
CoreCheckpoint、恢复状态和 reducer 的实现属于 `nola_agent_engine`，`nola_core` 复用/重导出其中的当前类型；Application 的具体 AgentItem/TaskUpdate 等
类型来自当前agent engine；Driver与Application的实际依赖以本页生产图为准，不能将无环或编译通过等同于全部业务边界验收。

## 当前耦合及其适用范围

- `nola_core` 是 rlib facade，不是当前原生最终 staticlib/cdylib 产物。最终原生产物由 `nola_mobile_bindings` 产生，并显式保留 Core 的纯辅助 ABI 符号。
- `nola_mcp_host` 已是共享 crate，CLI 与 mobile_bindings 都声明依赖它；它当前仍依赖 `nola_core`，不能写成独立于 Core 的纯 Domain adapter。
- `nola_application` 当前声明依赖 `nola_runtime_driver`。这条真实 manifest 边必须呈现；仅凭声明不能推断每个用例都在调用 Driver，也不能拿旧图证明两层已完全隔离。
- Driver 的通用 Host 执行循环使用 `HostExecutionProtocol`/`CommittedRuntimeApplicationPort`；同一 crate 的 `goal.rs` 仍引用 agent engine 的 Goal 类型及函数。因此“整个 Driver 无生产 engine 依赖”不是当前事实。它仍不拥有 Store 业务提交或另一个 reducer。
- CLI/mobile 对 Core facade 的引用、Application 对具体 engine 类型的依赖仍需按用例审查。模块图无环并不证明全部端口化或副作用隔离已经完成。

## 执行驱动的纯传输端口

`host_protocol::HostExecutionProtocol` 以关联类型绑定 Request、Response、BoundaryInput、
Event、Trace、Control。Agent engine/Core facade 的 `AgentRuntimeProtocol` 提供具体类型族；这描述通用 Driver 传输循环，不覆盖 `goal.rs` 等适配模块对 engine 的现存引用。Driver 不取得 checkpoint 提交或 reducer 所有权。
`HostDriver` 执行已提交请求，`CommittedRuntimeApplicationPort` 负责事务接受不透明响应与
边界输入。CLI SessionStore 实现具体 Task receipt/指导导入；Driver 只在提交成功后发布及 ACK。

Request/Response 跨 scoped Host worker 必须 Send；事件、control 与边界输入在调用线程消费，
无需把 Application、数据库连接或 Core 状态移入 worker。Driver 仅读取纯传输生命周期标记：
execution-start、terminal、model delta、permission suspension，维持租约续期、fence 和提交 ACK
顺序；这些标记不能自行批准工具或完成业务目标。

BoundaryInputSource.take_pending 可返回整批。接受失败或源 ACK 失败时，Driver 按原序归还
全部未 ACK 条目；已经提交但丢 ACK 的条目依赖 Application 精确身份去重，不重执行副作用。
`opaque_protocol.rs` 使用完全自定义非 Core 请求/响应、数字输入和内存 Application 端口，
覆盖提交 ACK 前不执行副作用以及失败批次保留。真实 SQLite e2e 继续作为 dev fixture；相应测试结果必须绑定具体工作树；本文只核对声明与源码，不将历史未运行记录当作当前暂停要求或通过证明。

## 自动门禁

复用 `cli/tests/dependency_boundary.rs`，不增加另一套 manifest 解析器：

```sh
cargo test --locked -p nola_cli --test dependency_boundary
```

现有 `.github/workflows/rust-build.yml` 已运行这个 integration target，新增测试随同进入该门禁。
仓库另有 `core/nola-domain/tests/workspace_dependency_architecture.rs` 的既有禁止边/源码检查，
覆盖既有禁止依赖与部分跨语言源码边界。这里仅记录门禁入口，本次文档核对没有重新运行测试。新增允许方向检查收紧未列出的反向边；两组不允许有相反规则。
检查使用 Cargo 原始 package name，覆盖别名、可选依赖、所有 target 声明、build dependency；
允许 dev fixture 使用具体 Store。缺失受保护包直接失败，不允许删除/重命名后悄悄跳过检查。

除了当前图符合性测试，还对 metadata 副本注入反例：Domain→HTTP、Kernel→Application、
Application build→Store、Store Port→SQLite、Store→Application，均必须被拒绝；
同一边改成 dev fixture 才允许。反例不修改真实 manifest，也不连接网络。

门禁检查的是依赖方向与部分能力引入，不是通用安全证明：
`std::fs`/`std::process`、动态加载、FFI、间接外部库能力、Host 内部用法仍需要源码及运行验收。
metadata --no-deps 不解析整个第三方传递图，文档不得据此声称纯函数性已经证明。
这些边界测试也不能证明授权、幂等、恢复、性能或五端 UI 正确。

## 后续结构整改的接受条件

1. 调整Application与agent engine边界时列出实际调用的输入/输出及所属端口，避免只更换转发层。
2. Client 的运行、控制、配置、存储都走 Application typed 用例；Host 只消费已准入效果。
3. 接口调整后同步收紧白名单并删除不再存在的边，不能永久保留旧边例外。
4. 图变化须连同模块契约、纵向测试和性能影响一起审查；门禁绿灯只是其中一个条件。

修改前的历史本机隔离 NOLA_HOME/mbx 验证：CLI dependency_boundary 3 passed，Domain
workspace_dependency_architecture 14 passed，均 exit 0、测试阶段各 0.03s。
这是本地历史执行，不是当前修改的通过证明或已观察的 CI 结果；没有跑整个 CLI 或五端端到端。

软件更新新增 `nola_update_host -> nola_store`（只读取当前格式常量，不打开数据库），CLI 与 mobile_bindings 依赖该 Host 组件。网络、验签和安装包暂存位于 Host；没有向 Domain/Kernel/Core reducer 添加网络或文件写入依赖。详细发布合同见[更新架构](SOFTWARE_UPDATE_ARCHITECTURE.md)。
