> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 分层与对象协作

核对：2026-11-08；分层约束与当前部署分开说明。术语统一见[软件架构总纲](SOFTWARE_DESIGN.md)，职责详见[Core/Application/Host](CORE_HOST_ARCHITECTURE.md)。

## 封装原则

### Agent reducer、checkpoint 与 trace 实现所有权

`nola_agent_engine` 是运行状态机的唯一实现 owner，不依赖 `nola_core`、Store、Application 或平台 Host。它持有 Agent 状态、权限续体、效果重放判定、checkpoint 和 trace 游标；输入是显式时间的 submission/Host observation 与冻结配置，输出是 typed transition、待提交效果及 checkpoint。Application 负责持久准入、事务提交与恢复证据核对，不能在 Host 中另建 reducer。Runtime Driver 消费已提交工作与控制协议，并包含 Goal/任务执行适配；不取得 Store 业务写权。

Agent engine 的 context/memory/tool-policy/prompt/hook 模块仅做确定性计算。Application C ABI、embedded runtime/数据库句柄及其序列化位于 `nola_mobile_bindings`。`nola_core` 仍提供若干纯辅助 ABI 的函数实现，bindings 通过 force-link 保留这些符号；Core facade 引用 engine 同一类型，不保存第二份运行模型。生产 Application 不依赖 Core 或 legacy adapter。Trace 协议及 reducer 随运行内核拥有，Application 仅负责 journal 持久化。

此步骤消除对历史 god crate 的宽重导出依赖，但 engine 内部的协议/parts/context 仍是同一运行聚合，进一步缩小投影读取能力需独立列出，不能据此宣称全部模块已完成细粒度端口化。

实现入口：`nola-agent-engine/src/reducer_port.rs` 的 `AgentReducerPort` 提供消费式候选 reduction、恢复及 checkpoint 端口；`AgentCore` 的运行私有状态仍不可直接改写。Application 提交链通过此端口接收候选，只有 Store 事务成功后才接纳为热状态。Core 原生适配的类型等同性由 `core/tests/agent_engine_type_identity.rs` 回归源码约束；依赖门禁禁止 engine 回连 Core/Application/Store，当前依赖图仍有 Driver→engine（Goal 适配）及 Application→Driver 的生产声明边，不能将目标隔离描述成已经禁止这些实际边。原生 ABI 测试位于 `core/tests/agent_engine_native_abi.rs` 并调用真实 Core adapter；engine 测试只验证纯计算，不导入 Core 源码。

对象封装自己拥有的可变状态和合法操作；值对象构造后不可变。
公开端口按使用者需要提供能力，不传递可任意改写的整个 Workspace、Config 或数据库对象。
纯 reducer 可以是函数和值类型；面向对象不要求把它改成持有 I/O 的 class。
相同业务类型只有一个所有者；wire、存储行和平台展示 DTO 是映射形式。

| 层/对象 | 拥有的职责 | 不允许承担 |
|---|---|---|
| Domain | 身份、资源、版本、领域不变量与结果类型 | 平台 I/O、选择窗口状态 |
| Core/Kernel | 运行、权限、任务、上下文的确定性决策 | 持久化、外部执行、UI |
| Application 用例 | 来源核对、准入、跨对象事务、幂等回执 | provider 协议、绘制和另一个 agent loop |
| Store adapter | 事务、引用完整性、索引和受限查询 | 按客户端名字制定业务政策 |
| Supervisor/Driver | 已提交工作的执行/资源/重试与取消句柄 | 改写 Goal 结果、授予工具权限 |
| Model/Tool/Platform Host | 协议适配、实际效果和原始观察 | 直接写消息或复用失效授权 |
| RunController | 一个 Run 的观察/用户控制，owner 固定 | 当前窗口选择和配置权威 |
| WorkspacePresentation | 选中、滚动、折叠、绑定投影 | 决定完成或复制整个业务状态 |
| DraftRepository | 设备草稿版本、不可变附件、条件保存 | 恢复时覆盖已提交会话配置 |
| Bootstrap | 创建实现、注入端口和生命周期装配 | 隐藏业务政策和绕过准入 |

## 当前物理部署

- 桌面/CLI/Rust Web 通过本地 Application Service 协调持久写入；Mac 另由独立 `nola-agent-host` 进程承载物理 Host 执行。它不是交互式 TUI，也不能用进程退出决定 Run 终态。
- iOS/Android 的 `nola_mobile_bindings` 以 embedded Application 复用相同用例，最终提供 staticlib/cdylib；同 canonical 数据库身份复用 runtime 并持有写者锁，不为每次 FFI 查询新开业务库。
- Worker/D1 是另一部署及数据域，处理认证、代理、渠道与 replica；不是桌面 Store 的替代写者。连接远端 Host 时，执行权属于准入绑定的源 Host，本地镜像不变成本地执行输入。

源码与差距见[当前实现映射](CURRENT_IMPLEMENTATION_MAP.md)、[依赖图](DEPENDENCIES.md)和[本轮源码核对](ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 部署与生命周期

### CLI 会话配置的模型与执行策略边界

会话 `modelSelectionID` 与对应 `modelOverrides` 是模型选择、采样及 thinking 选择的唯一 owner；执行策略只保存后端、外部执行 session/model/provider、SWE、复杂度 tier、指令、权限、压缩策略与资源预算，不能覆盖已解析模型参数。物理 context window 属于 per-model override。CLI create/persist/restore 使用同一解析流程及 caller CAS，不从上一会话继承外部 session。

CLI 首次显式模型配置加入当前 catalog；完整 thinking capability declaration 必须来自现有 catalog 或显式初始化声明，不能从一次 effort/budget 选择制造能力集合。未声明控制只允许 automatic；协议字段必须显式有效，不能把缺失 wire protocol 静默当 Chat Completions。凭据仅在首次显式初始化时写当前 vault，后续恢复必须解析已持久 credential ref。工具能力继续遵守当前 runtime/catalog 规则，不因本整改默认关闭。

当前 CLI 初始化声明为 `model_capabilities.reasoning_declaration`，使用 Domain `ReasoningCapabilityDeclaration`（`control=effort` 配 `values`，`control=budget` 配 `tokens`），验证后投影到 catalog source。会话只持久 `reasoningPreference` 与 `samplingOverride`：Responses 保留明确 effort；Chat/Anthropic 必须精确映射声明的三档，无法映射的预算拒绝写入，不调整预算。ExecutionPolicy 不再包含 `reasoning_selection` 或 `sampling_override`。

一个进程可复用模型权重、连接池、内容缓存和只读目录；会话配置、Run 参数、取消和授权范围必须显式隔离。
registry 的全局锁只查找句柄；长 I/O、模型流、用户确认不持有跨会话全局锁。
同 Run 事件有序，不同 Run 并发受显式资源策略约束。
窗口可以停止观察而 Run 继续；若系统回收移动进程，持久工作转为可恢复/受阻，不承诺平台不允许的后台常驻。

checkpoint 使用明确命名域；禁止 flatten 多份可变结构到同一层造成字段冲突。
完整消息/任务事实、执行恢复和展示投影各有边界，不能为了恢复方便复制出第二个可写权威。
子代理只得到范围化任务与结果端口；父取消向子传播，子取消不向父或兄弟传播。

## 验收

假 Host 验证纯状态转换，真实 Store 验证提交与回放，真实 Host 验证效果和取消；
平台与真实模型验收补齐调用链，不能互相替代。
依赖门禁约束允许边，但无法单独证明 std I/O、FFI、全局可变状态和动态加载的运行行为。
参考项目的旧比较与实现映射保存在[历史快照](../../历史任务/设计审查前快照/CORE_HOST_LAYERING_20261015.md)。
