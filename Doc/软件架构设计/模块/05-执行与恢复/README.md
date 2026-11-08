> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# MOD-05：Runtime Driver / Supervisor

状态：现行模块职责；2026-11-08 核对当前实现入口及依赖。规范要求、静态接线与历史运行证据分别说明，不将未复测路径标记为通过。

## 设计、输入输出与生命周期

- 输入：已提交 outbox、取消意图、Host observation。输出：claim/renew/result 等 Application 命令。
- 拥有：运行进程、连接、背压和临时句柄；不拥有 TaskOutcome 或业务历史。
- 每次 claim 必须有 worker generation 与有界 lease，旧 owner 结果只能按核对流程处理。
- lease 是防止重复所有者派发的栅栏，不是外部操作已结束的证明。
- 非幂等 effect 在派发后丢失回执：先 inspect；不可核对则 unknown_effect/blocked，不直接重跑。
- 验证：Host 退出、服务重启、迟到回执、续租失败、取消与重试竞态，逐项断言是否真正发生副作用。

### Host 进程内装配接口

这些接口细化 MOD-05/06，不新增业务所有者。当前源码位于 `cli/src`，已有
`nola-agent-host` 目标及不启用 `terminal-ui` 的构建声明；独立 launch/request/entry
负责 native stdin 执行。构建与依赖闭包须按当前产物单独验证，旧“本批未构建”不代表现行源码没有这个可执行目标。
`native_host_commands` 的 Server/session 控制入口仍使用共享 Config，Server 仍装配
Web 服务；这不允许执行层反向调用终端展示，也不等于 Web 已从该二进制中剥离。

| 接口 | 输入与输出 | 生命周期及禁止事项 |
|---|---|---|
| `HostInvocationRequest` | 产品解析后的模型、工具、会话、输入、执行、运行选项六块快照；转换失败返回配置错误 | 在调用前确定具体会话；不得携带 CLI 草稿/终端格式。worker 私有传输不继承父进程执行权，Debug 不输出凭据 |
| `RuntimeExecutionControl` | 执行身份、局部取消、typed 输入队列、固定会话 owner；输入返回本地/持久准入回执或错误 | 子级继承父取消但不能取消兄弟；回执不是消费 ACK。跨线程传 clone 后显式 enter，作用域句柄不跨线程 |
| `RuntimeInteraction` | 已判定需要询问的提示数据 → 选择、用户指导、返回输入框或传输不可用；持久问题 → 答案/取消/不可用 | 仅采集选择；不做风险分类、不写 grant、不授予执行权。无 adapter 必须不可用，不能转换成 Allow 或用户 Deny |
| `RuntimeOutputPort` | 已提交投影/游标、诊断与传输事件 → 客户端输出 | 输出成功不证明任务完成，输出失败不撤销提交；最终任务结果仍由 canonical seal/receipt 决定 |
| `process_observation` | PID → 是否仍可能存活 | 仅观察；未知保守保留，不替代 worker generation/lease，不证明 PID 启动身份 |

交互正常链路：Core 提出等待 → Application 提交等待身份 → Host 采集选择 →
Application 核验原会话/Run/决定身份后提交 → 读取已提交后继执行权。关闭提示、取消采集、
适配器不可用和连接断开都不产生批准或拒绝事实；原等待保留，可按同一身份重新呈现。
用户指导是单独的输入/指导记录，不能修改尚待批准的原参数后沿用旧授权。

进程重启只恢复 canonical 等待、控制意图与回执，不恢复 TLS、交互 adapter 或旧取消句柄。
同线程作用域乱序退出必须仅移除自身绑定，不能回灌进入时保存的旧 owner；正常退出、错误
返回及嵌套运行遵守同一规则。CLI 键盘监听由产品 RAII 句柄持有，Host 不启动监听器。

验收分别覆盖：无界面等待后恢复、跨会话拒绝、子级取消隔离、乱序释放不复活旧 owner、
批准提交后断线重放不重复 grant/执行，以及独立 Host 目标不编译终端展示模块。
Web 服务装配与 native 执行层的依赖须分别审计；当前目标的 Web 控制入口仍存在，
不能用无终端特性的构建结果证明同时完成了 Web 依赖拆分。
性能记录需分别测交互等待和提交/恢复耗时；用户思考时间不计入 Host 工具执行开销。
预算须来自同一链路真实基线，当前不以新增接口数量或未执行测试宣称改善幅度。

## 对象封装与依赖协议

跨模块边界以[架构决策与所有权](../../总体设计/架构决策与所有权.md)及[流程与恢复](../../总体设计/跨模块流程与恢复.md)为准，本文细化本模块职责。

每个 Run 独立监督器与取消作用域；进程句柄与会话持久数据分离。
所有跨边界输入输出遵循[公共信封与错误契约](../../模块契约.md)。对象命名为设计职责，不要求新增同名类型或 crate。
模块不得暴露可变内部状态；写入必须由其权威所有者处理。具体接口复用已有类型，缺少类型化边界列入整改，不用别名或额外包装宣称解耦完成。

## 当前实现入口与范围（2026-11-08）

| 源码 | 具体职责 |
| --- | --- |
| [lib.rs](../../../../core/nola-runtime-driver/src/lib.rs) | 已提交 Host request 的领取、接受和 ACK 循环 |
| [task_execution_authority.rs](../../../../core/nola-runtime-driver/src/task_execution_authority.rs) | worker 持有精确已提交任务租约的进程内权柄 |
| [native_host_entry.rs](../../../../cli/src/native_host_entry.rs) | 独立 agent Host 的 stdin 执行入口 |

通用 Driver 循环与同 crate 的 Goal/任务适配要分开描述；当前 Goal 模块仍依赖 engine。进程内 lease handle 只是对已提交授权的引用，重启不能据此恢复执行权。 逐模块差距与证据边界见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 相关设计

- [DURABLE_RUNTIME_DESIGN](DURABLE_RUNTIME_DESIGN.md)
- [HOST_COMPOSITION_BOUNDARY](HOST_COMPOSITION_BOUNDARY.md)
- [RECOVERY_AND_AUTHORITY](RECOVERY_AND_AUTHORITY.md)
- [WAIT_PROTOCOL](WAIT_PROTOCOL.md)
- [TERMINAL_PROTOCOL](TERMINAL_PROTOCOL.md)
- [HARNESS_CONTROL_SCHEDULING](HARNESS_CONTROL_SCHEDULING.md)

## 当前差距与验收

当前工作与验收范围见[任务入口](../../../当前任务/README.md)。此前阶段实施与验证见[阶段归档](../../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)；更早记录见[历史架构整改记录](../../../历史任务/2026-10-19_当前任务归档/架构整改主线.md)。
上述验证条目是验收要求；是否已经执行、使用的代码版本和证据必须另行记录。性能指标遵循[测量契约](../15-观测与验收/MEASUREMENT.md)，未测量项不得填写推测值。

## 当前恢复接口（2026-10-28）

child pump在启动和后继claim前复用Core到期delivery恢复与effect恢复准备。审批重示只恢复原pending请求，不等于Allow；界面/观察退出不提交业务取消。cancellation_only沿原bound Run推进已持久取消，不创建新模型尝试。

具体字段、返回值、错误和源码索引见[当前 Core 与 Host 接口](../01-领域与协议/CURRENT_RUNTIME_INTERFACES.md)。本次仅核对文档，不新增运行验收结论。
