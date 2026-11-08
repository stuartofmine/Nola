> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Run / Checkpoint / Effect 仓库切换（2026-09-08）

本切面迁移已经打开的本地 Run：Host 响应恢复与提交、活动输入提交、终态提交、可投递 Host effect 恢复。它没有移动旧 Core 状态机，也没有将 Application 的全部 Store 依赖改名后宣称删除。

## 实际迁移范围

`nola-store-port/src/run_effect.rs` 接管九个原 Store DTO：
`StoredCheckpoint`、`StoredActiveRun`、`StoredRunInvocationConfig`、`StoredOutboxEffect`、
`OutboxEffectInput`、`HostEventInput`、`RunEventInput`、`CommitTransitionInput`、`CommitTransitionResult`。
Store 保留这些名称的 Rust reexport，供未迁移的调用方过渡；Application 生产模块直接从 Port 引用它们，不经 Store reexport。

`RunEffectRepository` 定义 checkpoint/revision、active Run、invocation config、单 effect、同快照 Host delivery，以及原子 transition 提交。Port 依赖 Domain/serde，不引入 SQLite、Application 或旧 Core。

以下 `nola-application/src/lib.rs` 生产 helper 已实际改为 `R: RunEffectRepository + ?Sized`，不再接收 `&NolaStore` 或 `&mut NolaStore`：

- `load_run_invocation_config`、`validate_run_invocation_config_digest`。
- `load_committed_host_requests`。
- `accept_host_event`。
- `reduce_and_accept_host_response` 及其 `_with_core` 热状态版本。
- `reduce_and_commit_active_submission` 及其 `_with_core` 热状态版本。
- `complete_assistant_turn`。

`ApplicationRuntime` 两处 hot Run checkpoint revision 探测显式调用该 Port。`operation_result.rs` 的 transition 结果转换、`conversation_read.rs` 的已迁记录转换，以及 session checkpoint 解码参数均直接使用 Port DTO；没有只搬类型而继续从具体 Store 引入这些 DTO。

真实调用链为：

```text
ApplicationRuntime + 原有 receipt UoW
  → 泛型 Host / active submission / completion helper
  → RunEffectRepository
  → NolaStore 的同连接适配器
  → 原有 commit_transition SQL/savepoint
```

## 快照与事务

`nola-store/src/run_effect_port.rs` 委托现有同一个 `NolaStore` 连接，不另开读写数据库。`load_committed_host_delivery` 将 pending outbox、Run 所属 canonical conversation、canonical checkpoint 放在同一快照中返回。已有调用者 UoW 时复用它；没有外层事务时使用 `read_snapshot::pin` 并在读取完成后释放。无 pending effect 时不读取完整 checkpoint。

`commit_transition` 仍使用原有 Store 实现：revision CAS、Host inbox、Run events、Part mutations、checkpoint、outbox/ACK、Run 状态一起提交。嵌套于 receipt UoW 时仅释放 savepoint，不提交外层 receipt；错误也不越权回滚调用者的事务。

Application 的投递恢复还检查：canonical checkpoint 的记录 owner 对应 delivery owner、effect 对应请求 Run、Host envelope effect/request ID 对应持久化行、Core checkpoint 内部 session 对应其 state session、request session 对应该 Core checkpoint，以及 checkpoint pending journal 中的 request ID。恢复游标使用 checkpoint 中最后接受的 response sequence。

## 当前 Core session 与 canonical conversation 的身份边界

不能在本批迁移中假定旧 Core session ID 恒等于数据库 canonical conversation ID。实际生产入口 `ApplicationCommand::BeginAssistantTurn`（`application_command.rs`）调用 `begin_assistant_turn`，后者把 `command.core_config` 交给 `AgentCore::try_new/restore`，同时把独立的 `command.conversation_id` 写入 `BeginTurnInput`，没有二者相等的 admission 规则。CLI typed `begin_assistant_turn` 也直接提交这个命令。

旧 Core `agent/runtime/bootstrap.rs` 要求配置中存在 session ID；恢复时校验配置、checkpoint 和 state 内部的 session 一致。它不知道 canonical Store conversation 的 ID。此处不是说正常客户端必然产生不同 ID，也不是说缺少 Core session 会被自动补成合法值。

现有真实 Application fixture `cold_runtime_rebinds_the_active_request_before_caching_a_boundary_submission` 和 `application_runtime_broadcasts_small_deltas_before_durable_batch_commit` 分别使用 canonical `cold-boundary-conversation` / `batch-conversation`，而 `runtime_config()` 的 Core session 为 `recovery-conversation`。它们通过真实 begin helper 建立状态，不是 SQL 伪造。最初把这两个域强制相等的新增断言使这两项失败；最终按上节分别校验两个身份域，保留既有冷恢复行为。

这项既存分离不构成历史 JSON owner、LocalBranch proof、权限或工具重放的豁免。若将来要统一两个域，必须作为 Begin/恢复/Host 协议整体变更实施，不能在 delivery reader 中突然拒绝此前合法创建的 Run。

## 验证与范围

- `run_effect_port_e2e`：非 SQLite 仓库调用真实 Application helper 与真实旧 Core，加载 committed request，冷恢复模型 delta；原子提交失败保持 checkpoint/effect 不变；同请求重试后再次冷恢复 terminal response，ACK 后不再投递；错 Run/request 身份被拒绝。2/2 通过。
- `turn_transaction_e2e`：既有流式、active submission、NextTurn、终态与冷恢复回归 14/14 通过。
- 上述 receipt delta 测试增加最终 receipt UPDATE 故障。即使 Port transition savepoint 已释放，13 组事实仍全部回滚：conversation/message/Part、Run/checkpoint/effect、Host inbox/Run event/transition、sync outbox、projection cursor/event、receipt。之后同 Runtime 重试并关闭/重开重放，正文只出现一次。新增故障路径定向 1/1 通过。
- Store adapter 定向 1/1：外层 UoW 内创建 Run、读取 delivery、提交 transition；适配器不结束外层事务，整体回滚后 conversation/checkpoint/effect 均不存在；独立读取结束释放快照。
- `cargo check --workspace --all-targets --quiet` 通过（session 32980，exit 0）；Application `command_receipt` 单元回归 3/3 通过（session 18165）。
- 真实 Service `long_stream_keeps_real_service_rss_bounded_and_evicts_terminal_core` 1/1 通过（session 73071，21.61 秒，exit 0）：隔离数据库与真实 Service 子进程处理 2048 个模型 delta，校验 RSS、终态 Core 淘汰，以及复制到第二数据库后的接收端重启恢复。本项不是内存仓库或模拟 HTTP 测试。

上一切面结束时的剩余依赖（创建入口后续进度见下节）：ApplicationRuntime 仍持有具体 Store，BeginTurn/BeginInternalTurn/maintenance、历史恢复与 revert 等入口仍未全部泛型化；Application 的 `nola_store` 与 `nola_core` 正常依赖仍在。下一步应迁移 Run 的创建契约及这些实际入口，再将 Runtime 收敛到同一 UoW 的聚合仓库端口；不能用多连接或只加一个未被生产调用的 trait 代替。


root最终整合再次通过：Application lib83、run_effect_port_e2e2、turn_transaction_e2e14，
`/tmp/nola-run-effect-application-final.log`；全部exit0，无跳过。

## 后续切面：本地 Run 创建已迁移

`nola-store-port/src/run_creation.rs` 现接管 `BeginTurnInput`、`BeginTurnResult`、`BeginInternalTurnInput`、`BeginMaintenanceRunInput`、`BeginMaintenanceRunResult` 五个原 Store DTO。线格式不变，Store 仅保留过渡 reexport。`RunCreationRepository: RunEffectRepository` 提供三种原子创建操作：普通 turn、依赖已存在可见 turn 的 internal turn、无新增会话消息的 maintenance Run。

`begin_assistant_turn` 的普通与 internal 两条生产分支已使用泛型 Port；冷启动历史还要求 `CanonicalHistoryRepository`，由同期历史读取迁移提供，不能因脱离 SQLite 而跳过历史恢复。`begin_maintenance_turn` 已泛型化；`query_claimable_memory_jobs` 和 `build_memory_worker_plan` 直接使用已有 `RunEffectRepository`。Application 创建结果转换直接引用 Port DTO，不从 Store reexport 借道。Runtime 与真实 ApplicationCommand 分派继续调用这些 helper，没有另造仅供测试的业务实现。

Store 适配器在 `run_effect_port.rs` 委托同一 Store 实例已有创建事务；不另开连接，不提前提交 receipt 所有的外层事务。普通创建的 checkpoint、Run、消息/Part、outbox 与最终 receipt 仍共提交。原 internal trigger 与 maintenance 不制造可见消息的规则未改。

新增与扩展实证：

- 非 SQLite fixture 现在 3/3：新增调用真实 `begin_assistant_turn`，创建提交失败不留下 Run；重试成功；两次均访问空 canonical history Port；随后实际 Host response helper 从持久化 checkpoint 冷恢复并提交，活动 Run 阻止重叠创建。
- `turn_transaction_e2e` 现在 15/15：新增真实 Runtime 创建命令的最终 receipt UPDATE 故障，13 张事实表逐值回到基线；同命令重试成功，关闭/重开后 exact receipt 重放不产生任何重复事实，Run 的 Host delivery 可恢复。
- Store adapter 1/1 的外层事务回滚测试现通过 `RunCreationRepository::begin_turn` 创建。
- `ledger_e2e` 15/15、`turn_append_sync_e2e` 5/5：维护与内部创建的现有 SQL/复制回归改为显式调用新 Port，覆盖维护 Run 无新增会话消息与内部 turn 复制行为。它们仍是实际 SQLite 回归，不能称为非 SQLite maintenance 测试。

当前 Runtime 仍明确持有 `NolaStore`，Application 仍依赖具体 Store；本轮没有用别名或聚合包装隐藏它。上述创建入口已经脱离具体参数，下一完整切面是将各已使用的仓库 Port 与原子 command UoW 组合为 Runtime 的实际依赖，同时继续保持一个事务所有者。

本轮最终 `cargo check --workspace --all-targets --quiet` 通过（session 41206，exit 0），含同期历史/Permission/Skill Port 整合的当前工作区。
