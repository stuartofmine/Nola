> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Delegated permission 控制投递审计（2026-09-09）

本文件记录独立 DelegatedInvocation 账本接通后的下一项缺口。此轮仅审计源码并提出下一步契约，没有实现控制投递账本，也没有新增测试。当前 delegated 输出逐事件提交 ACK，不能作为权限控制消息已被 worker/backend 观察的证据。

## 当前实际链路

1. Core 在 `core/src/agent/runtime/tool_driver.rs:693`（已存在权限规则）或 `:1237`（用户明确授权）生成 `AgentEffect::ResolveDelegatedPermission`。原始请求由 `PermissionContinuation::DelegatedAgent` 保存 target effect、backend、session；参见同文件 `:723`。用户授权本身先进入 Application 的 canonical 提交。
2. `core/src/agent/runtime/effect_dispatch.rs:415` 将控制 effect 转换为 Host 请求。`core/nola-host-protocol/src/lib.rs:449` 的 payload 包含 `target_effect_id`、`backend_id`、可选 `backend_session_id`、`permission_request_id`、类型化 `resolution`，但没有目标 invocation UUID、权限 occurrence ID 或 worker generation。
3. `core/nola-application/src/delegated_invocation.rs` 中 `is_delegated`、`prepare_effects`、`validate_response` 和 `bind_delivery` 只为 `HostRequest::DelegatedAgent` 建立独立 invocation 身份。`ResolveDelegatedPermission` 的 envelope 不拥有 delegated invocation；也不存在另一个被验证的 typed target invocation 引用。不能直接给控制 envelope 填目标 UUID，因为那会混淆控制 effect 自身与被控制的 invocation。
4. CLI 在 `cli/src/agent_host_driver_parts/tool_projection.rs:2224` 处理控制请求：按 target effect 查找 worker；比较原始 backend 名称；明确忽略 `backend_session_id` 和 permission instructions。这里不检查目标 invocation UUID、worker generation、控制请求与 worker 的 session/turn/lease 关联，也不确认 worker 正在等待哪一个 call/RPC。对 Host 自身已有的通用 catalog/host 检查，不能替代这些目标检查。
5. 同文件 `:2281` 将 `{request_id, decision: String}` 放入 worker 控制 channel，随后立即在 `:2286` 发出 `Completed { permission_resolution_delivered: true }`。成功入队尚不能证明 worker 接收，更不能证明 backend 收到回复。
6. `cli/src/agent_host_driver_parts/orchestration.rs:1557` 的 worker 只比较 backend permission request ID。匹配则将字符串决定转换为布尔授权；不匹配则在 `:1567` 静默丢弃，控制发送方没有对应 ACK。实际回复 backend 的写入发生在 `cli/src/external_executor.rs:949`（OpenCode）或 `:1403`（Codex）。
7. 输出事件的 ACK 是另一条方向：`orchestration.rs:1298` 等待 Application 提交，`tool_projection.rs:1563` 在提交后 ACK。该机制没有覆盖控制输入。

行号是此次审计时位置，后续编辑应以符号名查找为准。

## 已证实的边界缺口

### 目标与权限请求身份

- 控制路径没有把新独立 invocation 账本作为目标权限身份的校验来源。
- backend session 字段存在于 wire，却在 CLI 控制分支被忽略。
- worker 仅凭 backend request ID 匹配；缺少与 invocation、原始 call、当前权限 occurrence 及 worker generation 的完整绑定。RPC ID 不是跨 session/generation 的永久唯一身份。
- 错误 request ID 可被静默丢弃，而控制 effect 仍已报告 Completed，之后 drain 甚至可能继续等待没有对应进展的 worker。

### 提交、观察与副作用不是同一事实

控制消息先入队，后提交 Completed。worker 可以在 Completed 提交之前取走决定并写入 backend。若提交失败，当前代码会设置 admission failure 并回收进程，但不能撤回已经写出的授权，也不能声称外部副作用已被撤销。

另一个窗口是 Completed 已提交，worker 尚未读取控制消息时退出。canonical 状态报告送达，却没有 worker 观察证据。即使增加 channel 接收 ACK，也只能证明 worker 接收，不能证明后续 pipe 写入或 backend 应用。

把 Completed 简单挪到 send 之前不能修复：它把问题改成“已完成事务，但尚未发送即退出”。把 Completed 放到 pipe write 后同样留下“写入成功、提交失败”的不确定窗口。本地数据库事务不能与外部进程写入组成原子事务。

### 重复控制

正常已确认完成的 outbox 不会自行重新加载；不能把该路径描述为必然重复执行。但并发持有的旧请求、重复 dispatch 或不确定投递重试，缺少控制执行前的独立去重记录。重复消息可先再次入队，Application 到响应阶段才识别重复。

第二条队列消息可能在下一次权限等待中被丢弃；若 backend 重用了相同 request ID，还缺少阻止旧决定命中新 occurrence 的绑定。因此当前证据既不能保证控制恰好发送一次，也不能直接断言每次重复都会导致 backend 执行两次。

## 建议下一步契约

### 独立目标及持久化投递记录

新增类型化 `DelegatedPermissionDelivery`，由 Application/Kernel 从 canonical invocation 和 pending permission 生成，不从提示词或关键词推导授权：

- 稳定 delivery ID；所属控制 effect 与控制 request ID。
- 目标 Run、invocation UUID、effect、backend、canonical backend session、worker generation/lease。
- 独立 permission occurrence ID，原始 backend RPC ID、call ID、完整权限请求摘要。保留原始结构化请求供检查，摘要不能取代请求本身。
- 类型化授权决定及已提交的授权来源/范围。补充指导继续保留为指导，不因其存在就转换为授权。
- 状态、revision、时间及具体投递证据。

同一 delivery ID、相同内容的重放返回既有证据；相同 ID、不同目标或决定必须拒绝。同一权限 occurrence 的冲突决定不能作为第二条无条件控制消息发送。

### 分阶段证据与提交 ACK

建议显式区分：

- `Prepared`：授权及控制 intent 已提交，但没有执行证据。
- `Claimed`：指定 worker generation 验证完整目标和当前 pending permission，并声明接受该 delivery。claim 必须提交并 ACK 后，worker 才能获得写 backend 的许可。
- `TransportWritten`：实际协议回复成功写入并刷新。它仍不是 backend 已经执行的证据。
- `BackendAcknowledged`：仅在 backend 协议提供真实确认或可验证后续关联事件时记录。
- `Interrupted` / `OutcomeUnknown`：worker 丢失、写入与提交之间崩溃、不能证明是否送达等。

控制输入需有独立 claim/ACK 机制，不能借用任意后续 text/tool 输出作为授权送达证明。只把“在指定证据层级完成”作为控制结果；不得以笼统的 `permission_resolution_delivered: true` 混合入队、接收、写入、应用四种含义。

父 invocation 的 `WaitingPermission` 状态及恢复行为应关联具体 permission occurrence 和控制投递证据，而不是仅靠下一次普通输出推断授权已经完成。

### 恢复与实际能力限制

数据库提交后、外部写入前，以及外部写入后、证据提交前，仍可能崩溃。若 backend 支持幂等 delivery token 或查询确认，可用真实协议证据恢复；若不支持，应保留不确定状态，禁止自动重发原授权或重放原始业务输入。warm worker 可通过同 generation 的已消费 delivery 记录返回证据，但不能把该内存记录当成跨进程持久证据。

当前 CLI in-process runtime 在 `cli/src/application_port.rs:96` 通过普通 `application.execute` 提交运行时命令；`requires_editor_admission` 只把部分编辑命令送入 receipt 路径。Service/received-command 的 receipt 事务可以保证 canonical intent/claim/证据与 receipt 一起提交，却不能自动使 backend 写入具备 exactly-once 语义。下一阶段要分别验证普通提交和真实 receipt 路径，不能用一个 CLI 测试冒充两者覆盖。

## 建议的真实回归场景

1. **完整目标匹配。** 保持 backend 名称和 RPC ID 相同，分别替换 invocation UUID、backend session、worker generation 或原始 call；通过真实进程记录输入字节，确认所有错误目标在 backend 收到回复前拒绝。另验证旧 RPC ID 在新 permission occurrence 中重用时不会消费旧授权。
2. **确定的崩溃/提交边界。** 用显式同步屏障控制 claim 提交、worker 读取、pipe 写入和最终证据提交。claim 提交失败应零 backend 写入；写入后证据提交失败必须形成可恢复的不确定状态，冷恢复不擅自重发。对真实 received-command 路径追加 final-receipt 提交故障，证明回滚范围；不将其冒充对外部副作用的回滚。
3. **重复与冲突控制。** 同一 delivery 重放返回相同证据，真实 backend 最多收到一次匹配回复；不同 delivery 对同一 occurrence 的冲突决定被拒绝。随后发起第二个权限请求，确认不会读取第一轮残留授权；错误 request ID 也不能被报告为成功送达。

这些场景是待实现、待验证的门槛。本审计不声明权限控制投递已经完成 V2 切换。
