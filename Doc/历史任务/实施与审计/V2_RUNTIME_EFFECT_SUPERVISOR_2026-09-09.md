> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 Runtime Effect Supervisor 第一切片（2026-09-09）

本切片把通用 Effect Delivery 从“调用方顺便 pump”推进为进程级常驻调度链路。它延续
`ARCHITECTURE_V2.md` 的所有权约束：Application Service 仍是唯一 canonical Store 写入者；
Supervisor 只读取 Application 给出的已提交工作计划、调用 Host，并通过现有 typed Application
命令领取、续租和提交结果。Supervisor、Service Client 和 CLI Host 均不直接打开 canonical Store。

## 已完成的边界

- Store schema 21 提供按 `conversation_ids` 限定的跨 Run 工作计划。单次 scope 和结果均最多
  64 项，每个 Run 只返回一个稳定排序的 head；Pending、Retry、Resume 与过期 lease recovery
  使用显式原因，未到期 lease 只贡献最近 `next_wake_at_ms`。
- Application 暴露只读 `LoadRuntimeEffectWorkSchedule`，再次检查数量与 conversation scope，不执行
  Host，也不产生命令收据。
- Application Service protocol 19 提供 `runtime_effect_work_wait`。Service 在持有 Application
  mutex 时注册提交唤醒，随后释放 writer；新提交、租约 deadline、客户端 deadline 或断连会结束
  等待。空闲期间不定时查询数据库。
- Service Client 提供同步与可取消的 typed wait。取消会主动 shutdown 当前 Unix socket，连接建立
  阶段也按短 poll slice 检查取消。响应必须通过完整 request scope 校验；传输预算为服务等待时间
  加一秒有界宽限，避免 30 秒边界竞态。
- 新增独立 `nola-runtime-supervisor` crate，负责 conversation 注册、dirty scope 重建、单 worker、
  每 conversation 指数退避、长等待取消、drain 和 shutdown join。注册 scope 原子限制为 64；退避
  中 conversation 的最近恢复时刻会缩短其他 scope 的长等待，毫秒 deadline 使用向上取整。
- Runtime cancellation 支持一次性线程安全回调。CLI adapter 将它绑定到现有
  `ExecutionCancellation`，因此 shutdown/unregister 会进入 provider、工具、delegated session 和
  子进程使用的同一取消树。
- CLI 仅在持久会话模式启动 Supervisor。Host 获取、替换、淘汰与失效更新注册 scope；worker 按
  conversation 和 Run 双重身份路由到 warm Host，`try_lock` 与前台执行维持单消费者。后台 Emitter
  静默，客户端从 Application projection 读取 canonical 展示。

## 已验证的故障与时序

- scope 外更老 pending work 和更早 lease deadline 不会遮挡已注册会话；动态 SQL 只拼接占位符，
  conversation ID 全部参数绑定。
- 提交会提前唤醒真实 Service IPC wait；未来 lease 到期会返回 RecoverExpiredLease；断连会释放
  waiter，且这三条路径均不写查询收据。
- dirty registration 会取消旧 wait 并按新 scope 重发；超过 64 项被原子拒绝。
- 同一 Supervisor 只运行一个 Host worker；某 conversation 失败只退避该 conversation，不阻塞
  其他 conversation，并会在约定 deadline 后重新进入 scope。
- unregister 只取消目标执行；drain 完成当前工作后退出；shutdown 取消当前物理执行并 join。
- 现有 Effect Delivery 的真实 HTTP 静默首字节、真实子进程取消、按 Effect 隔离续租失败与最新
  lease ACK fencing 继续适用。

本轮全量证据：Store 279、Application 306、Protocol 32、Service 76、Service Client 25，共
721 项通过；Runtime Supervisor 9 项通过；CLI Supervisor 专项通过；workspace all-targets 编译
通过。CLI 完整 bin suite 仍保留此前工作区基线的 14 项失败，不能表述为全量 CLI 已绿。

## 仍未完成

该切片建立了 CLI 进程内的常驻调度与恢复基础，但还不是整个 V2 Runtime 完成：

- macOS、iOS、Android 和 Web 尚未全部装配同一 Supervisor 生命周期；冷启动时只有进程已注册且
  能重建 Host 配置的 conversation 可进入当前 scope，durable Host 配置解析仍需收敛。
- delegated permission 的长静默续租/拒绝故障证据仍需补齐。
- OutcomeUnknown 尚无基于外部证据的人工处置命令；外部系统没有幂等 token 或查询 ACK 时不能
  声称 exactly-once。
- RuntimeTask 仍需删除泛化 upsert，完整迁移到 transition-specific claim/renew/report/suspend/
  complete 命令与监督器调度。
- 完整 Core/CLI/Android 所有权、Device 配置绑定，以及主文档第 20 节的五端、真实模型、长任务和
  千万字符 UI/性能验收仍未完成。

## 后续版本状态

第一切片完成后的人工未知结果处置又把Store schema提升到22、Application Service protocol提升到20。
`OutcomeUnknown`目前只支持有完整执行身份和证据的`confirmed_not_applied`授权重试；其它人工结论仍需
Core显式resolution事件。RuntimeTask随后开始移除full-row upsert并迁移到独立attempt身份，因此读取
本文中的21/19应理解为Supervisor第一切片当时的版本，而不是工作区最新版本。

截至2026-09-10，RuntimeTask attempt/renew/progress继续推进schema和protocol硬切。Supervisor worker
outcome现为typed `Drained` / `RetryableUnavailable` / `BlockedConfiguration`。Busy与暂时服务故障进入
可重试退避；缺少当前进程配置或冻结Host binding不匹配明确阻塞，只有实际Drained才清退避。

CLI/Web常驻进程已有最小cold Host factory：每个持久Host获取时按conversation保存当时已解析的进程内
`Config`，启动时只为显式`session_id`预注册启动Config。Unregistered或StaleRun先从Application读取同一
active Run和checkpoint，以checkpoint内非秘密`HostInvocationBinding`精确核对当前endpoint、credential
摘要、模型生成参数和执行策略；核对完成后才复用既有`resume_agent_core_checked`/Runtime Driver进入
claim和dispatch。其它conversation不会套用单一启动Config，而是返回`BlockedConfiguration`。Service
不可用或active Run竞态返回`RetryableUnavailable`。checkpoint不会保存明文credential，因此一个没有
当前凭据的全新进程不能自行重建Host，这一限制不通过持久化secret或猜测配置绕过。

非Unix wait transport仍未实现。跨进程真实Service冷恢复与旧lease/单terminal综合验收仍需补齐；现有
真实HTTP组件验收覆盖warm Supervisor drain，Effect Delivery专项继续覆盖旧lease fencing和terminal ACK。

### Worker outcome 收紧

Supervisor worker 已从无语义的 `Result<(), String>` 收紧为 typed outcome。只有明确的 `Drained`
会清除对应 conversation 的退避；`RetryableUnavailable` 表示当前进程没有可用 Host、Host 正忙或
已注册 Host 不拥有计划中的 Run，并进入同一个有上限的 per-conversation 指数退避。真正的执行错误
仍保留在 `Err` 通道，未与“暂时没有执行者”混为一类。

CLI adapter 不再把 `Unregistered` 或 `StaleRun` 当作成功，`Busy` 也属于可重试不可用。这样 durable
schedule 持续返回同一 head 时不会形成忙循环，并且一个 conversation 缺少 warm Host 不会阻塞同批
其他 conversation。确定性测试覆盖不可用 scope 在退避期被排除、deadline 前不重复查询、健康 scope
继续执行，以及退避结束后不可用 scope 重新进入计划。CLI 另以纯分类回归锁定 Busy、Unregistered、
StaleRun 和 Drained 的映射。本次专项结果为 Runtime Supervisor 10/10、CLI Supervisor 4/4。

这只是冷恢复的安全前半。当前 CLI worker 仍只能查找本进程内已注册的 Core Agent Host；durable Host
配置解析和 cold Host factory 尚未实现，因此进程重启后的任务仍不能宣称已经恢复执行。
