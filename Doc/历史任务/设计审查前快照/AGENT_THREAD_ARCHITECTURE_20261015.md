> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Agent Thread Architecture

> 2026-10-15 整体设计复核前的历史快照，仅保留当时叙述与证据。旧 ABI、Trace 权威、schema 数字、传输别名和其他旧规则均不作为现行规范。当前入口见[软件架构总纲](../../软件架构设计/总体设计/SOFTWARE_DESIGN.md)。

> 本文是 Agent 运行时专题合同，服从 [ARCHITECTURE.md](../实施与审计/ARCHITECTURE.md)。其中
> “Core 拥有 transcript”表示 Core 在一次 transition 中拥有消息语义和运行投影，
> 不表示 Core checkpoint 是完整会话的永久数据库。完整消息由 Nola Store 持久化；
> checkpoint 最终只保留 ledger cursor、活动 Run 和恢复所需状态。

## 目标

Nola Agent Core 采用与 Codex CLI 相同的关键边界，但不复制其平台实现：

- 一个会话对应一个 `AgentRuntime`。
- `AgentRuntime` 是唯一公开的可变入口。
- Core 决定会话、轮次、消息 Part、权限、工具调度、上下文压缩和恢复。
- Host 只执行模型、工具、权限 UI、上下文读取和 checkpoint 持久化。
- UI 只投影带序号的 Core Event，不从 provider chunk 或工具回调构造第二份状态。

## 所有权

| 层 | 持有 | 不允许持有 |
| --- | --- | --- |
| `AgentRuntime` | Session、Turn、mailbox、Part、permission rule、context window、pending effect | 网络、文件系统、系统权限 UI |
| `AgentRuntime` | 确定性状态转换 | 平台回调、异步任务、UI 状态 |
| Runtime Driver | 已提交 effect 执行、进程与连接生命周期 | reducer、未提交队列、Agent 决策、消息排序、权限策略 |

CLI 与 Web Host 在进程内按 `session_id` 注册长生命周期 `CoreAgentHost`。每个实例只
持有 `CommittedRuntimeDriver<PlatformDriver>`、冻结配置和已提交 checkpoint 的只读
投影；它没有 reducer 或待执行内存队列。普通用户输入由 Application 从 Store
checkpoint reduce，并原子提交 Message/Part/Run/checkpoint/Outbox 后，Runtime 才能执行。
不同会话分别加锁，禁止用全局运行锁串行化所有用户。
| Client UI | Event projection、交互输入 | provider 原始流、影子 turn、影子 tool state |

## 唯一写入协议

外部只能提交 `AgentSubmission`：

- `user_input`
- `permission_response`
- `host_response`
- `update_host_catalog`
- `update_session_configuration`
- `host_disconnected`
- `resume_host_effect`
- `interrupt`
- `compact`
- `reset`

内部 `AgentCommand` 与 `AgentEffect` 均为 crate-private。平台不能直接 dispatch reducer 命令。

## Session 与 Turn

输入分成两个不可混淆的通道：

- transcript items：用户输入、助手输出、工具调用与结果，写入 Session 历史；
- turn context items：本轮冻结的数据源、指令和环境上下文，只进入模型可见上下文，
  不写入会话历史，也不在下一轮自动重放。

每次模型请求都由 Core 使用同一份 Turn Context 与当前压缩后的 Session transcript
重新组装。工具回合不会重新抓取或替换 Context，确保一次 Turn 内的决策前提稳定。

原始 `ContextComponent` 也属于 Turn 快照。Core Context Engine 在 Turn 开始时完成：

- protected data authorization 与 freshness 门禁；
- component priority、platform rank 与 token budget；
- source metadata、component metrics 与 payload digest；
- 物化为实际 model-visible Items。

Checkpoint 同时保存来源组件和最终快照；恢复时不得重新读取数据源或重新解释权限。

`AgentSessionState` 保存跨轮次状态：

- checkpointed `AgentSessionConfiguration`
- 只追加、不被压缩覆盖的 `transcript`
- 面向下一次模型采样、允许压缩和裁剪的 `model_context`
- 当前 Turn
- steer mailbox
- next-turn mailbox
- 会话级权限
- context-window state
- Host catalog
- pending Host effects
- event cursor 与 idempotency key

`AgentTurnState` 只保存当前轮次状态：

- turn id 和状态
- 创建 Turn 时冻结的 `AgentTurnContext`
- model request 计数
- tool lifecycle
- open stream Part
- current-turn queued input
- doom-loop signature

终态 Turn 可以保留用于投影，但不阻塞 idle-only 的会话配置更新。

`AgentTurnContext` 固定当前 Turn 使用的模型、工具 schema、上下文窗口、Host id、
catalog revision 和 availability epoch。Host 或用户在 Turn 运行期间提交的新配置只影响
后续 Turn；已经发出的模型请求和工具事务不能被中途换模或换 Host。

## Steer 与 Next Turn

运行中的输入必须显式选择 delivery：

- `current_turn`：进入 steer mailbox。不得改写已经发出的模型请求；在下一个模型边界优先消费，消费前不得发出 `TurnCompleted`。
- `next_turn`：进入 session mailbox。当前 Turn 完成后自动创建新 Turn。

这两个队列不能由 UI 合并，也不能在 Host 重连时丢失。

## 会话配置

模型、工具 schema 和上下文窗口是 checkpoint 的一部分。Host 不得通过“用新配置恢复旧 checkpoint”隐式更改它们。

配置变化必须提交 `update_session_configuration`，且只允许在线程 idle 时执行。Core 随后：

1. 验证工具 schema 和窗口配置。
2. 原子更新 session configuration。
3. 发出 `session_configuration_updated`。
4. 生成 `configuration_changed` checkpoint。

## Part 与流式输出

Core 为 text、reasoning、tool、artifact、step 和 compaction Part 分配稳定 ID。

每个公开 `PartUpdated` 必须携带“该次 reducer 更新刚完成时”的不可变 Part 快照。
同一个内部事件产生多次更新时，不得为所有更新读取最终状态，否则 UI 会看到状态跳跃、
闪烁或旧消息被新消息覆盖。
Host response 只携带相关 effect、sequence 和增量，Core 负责：

- `PartUpdated`

UI 必须在首个 `PartUpdated` 时按稳定 Part ID 创建或原位更新容器。内部
`RuntimeEvent` 不跨 ABI，最终消息不得替换已经流式投影的消息。

工具完成时 Core 同时形成两个视图：`transcript` 保留完整输出或稳定
`full_output_ref` 与 artifacts，`model_context` 只保留按策略截断的模型可见预览。
压缩不得破坏 tool call/result 配对，UI 也不得从日志重新推断工具状态。

## Host effect

Host Catalog 是能力可用性、权限域、取消方式、重放安全性的可信来源；模型返回的
permission scope/rationale 一律不可信。Host 可以根据真实环境判断事实，例如命令是否
仅在允许目录内只读；Core 根据该事实和 Session/Turn 权限规则决定 allow/ask/deny，
并持久化权限 Part 与 Trace。Host 不得自行绕过 Core 建立第二套权限状态机。

每个 effect 包含：

- effect id
- request id
- session/turn owner
- Host id
- optional lease id
- catalog revision
- availability epoch
- capability id
- optional call id
- response sequence
- replay metadata
- `dispatched / accepted / streaming / cancelling` state

Host response 必须通过 effect/request id、Host lease、session/turn owner、catalog
revision、availability epoch 和单调 sequence 校验。重复、过期或错误 Host 的
response 不得进入 Session。

远端或跨进程 Host 还必须发布带 `lease_id`、签发时间、最后心跳和过期时间的 lease。
Core 只验证 lease 与 epoch 单调性，不读取系统时钟；Host 负责 heartbeat timer，并在
超时后提交 `host_disconnected`。进程内 Host 可显式使用无 lease 模式。

## Checkpoint 与恢复

当前 checkpoint schema 为 `8`。状态使用明确的 `conversation`、`execution`、`host`、
`journal` 命名域，不把多个可演进结构 flatten 到同一 JSON 对象。除会话、上下文和
pending effect 外，还持久化当前
Turn 的 Trace identity 与下一 sequence。恢复要求：

- session identity 一致
- checkpoint schema 一致
- model、tools 和 context-window contract 一致
- Host catalog 不倒退
- pending effect 不迁移到错误 Host

自动恢复只重放明确标记为安全的 effect。显式恢复保留 effect id、生成新的 request
id，并绑定当前 Host lease；Catalog 更新不得批量重写 pending effect owner。其余
effect 保持 pending，由 Host 或用户显式恢复。

## 平台约束

- CLI 与 `nola-agent-host` 复用同一个 Core Host loop。
- macOS 非宿主机交互只通过 `nola-agent-host` 协议进入 Core。
- iOS 与 Android 使用 v2 FFI Submission/Event/HostRequest 协议。
- 平台不得保留 v1 Agent runtime、影子 turn、provider chunk 消息排序或独立权限状态机。

## 参考项目吸收范围

从 Codex CLI 吸收：

- Thread facade
- Session/Turn 生命周期与不可变 TurnContext
- Submission-only mutation
- steer/next-turn mailbox
- model-boundary steer、采样前压缩以及 task/effect/checkpoint 边界

从 OpenCode 吸收：

- stable message Parts
- tool argument streaming
- last-match permission rules
- repeated tool batch loop guard
- tool pending/running/completed/error 状态与完整事务保留

从 Claude Code 吸收：

- safe-read parallel batch
- mutation barrier
- deterministic compaction boundary
- prompt-too-long 重试、最近 API round 保留和后台工作恢复信息

从 Open WebUI 吸收：

- socket/Host session lifecycle、heartbeat 与超时回收
- capability availability epoch
- request correlation
- reconnect cleanup

参考只用于机制设计。Core 不引入参考项目的 UI、provider 或平台耦合实现。
## Long-term memory policy

Long-term memory behavior is part of the checkpointed session configuration,
not an environment-variable side channel. `MemoryPolicy` controls whether
cross-conversation tools are model-visible, whether boundary-safe automatic
extraction is enabled, recall limits, confidence thresholds, and optional age
limits.

Durable memories use a structured record with scope, kind, content, summary,
tags, confidence, importance, sensitivity, expiry, supersession links, and
provenance. Core owns these semantics; Hosts own storage and retrieval I/O.
Expired records are excluded from recall, and newer facts can explicitly
supersede older records rather than creating an unresolvable duplicate list.
