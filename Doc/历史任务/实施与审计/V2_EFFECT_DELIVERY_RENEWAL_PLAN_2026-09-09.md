> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Effect Delivery 租约续期接线方案

日期：2026-09-09\
状态：通用续租链路与真实 HTTP/子进程长等待验收已完成；delegated/permission 长等待仍待补充

## 原始缺口

`EffectDeliveryAttempt` 已有 `heartbeat_at_ms`、`lease_expires_at_ms` 和 revision；Kernel 已能
校验 Leased/ExecutionStarted 状态、generation/owner/token、未过期及不缩短租约，Store 也支持
同状态 CAS 更新。但 Application、Runtime 和 Host 没有续租入口。Runtime Driver 在 claim 时固定
发放 300 秒租约，然后同步阻塞于 `AgentHostDriver::dispatch_batch`。

因此只在 Host response 中增加 heartbeat 不足以解决问题：HTTP 建连/首字节、同步工具、子进程、
lifecycle handler 和 delegated output 都可能在超过租期的时间内没有任何 response。Runtime 此时
拿不到 Application port，最终响应会被过期 fence 正确拒绝。

## 采用的边界

续租是 Supervisor 对 Application 的控制命令，不占用 Core Host 业务响应序列：

```text
Runtime main thread                 scoped Host worker
        |                                  |
        | claim                            |
        |--------------------------------->| dispatch request
        |<----- response + one-shot ACK ---|
        | accept response transaction      | waits
        |----------- durable ACK --------->|
        |                                  | external execution
        | heartbeat CAS                    | may be completely silent
        | heartbeat CAS                    |
        |<----- terminal + one-shot ACK ---|
        | accept terminal transaction      | waits
        |----------- durable ACK --------->|
```

Application 新增 typed `HeartbeatEffectDelivery`，输入 run、attempt、generation、owner、token、
调用者当前 expiry、heartbeat time 和 requested expiry，返回完整的新 `EffectDeliveryLease`。
它加载唯一 active Attempt 和 pending Effect/checkpoint，调用 Kernel heartbeat，并通过 StorePort
在一个事务内 CAS 提交 Attempt 与 Run event。它不调用 Core reducer，也不改变业务 response cursor。

Runtime 只有在 `ExecutionStarted` 得到 durable ACK 后才注册 heartbeat。静态拒绝、deadline 拒绝
和 start ACK 失败都不会续租。Host worker 发出的 response 必须保持 attempt/generation/owner/token；
Runtime bridge 只允许把已验证的旧 expiry 更新为自身最近一次收到 ACK 的新 expiry，再送 Application。
各 adapter 不得自行生成 token、写 Store 或猜测续租已经成功。

## Kernel 与并发规则

- claim 和 heartbeat 的 `expiry - now` 均不得超过统一最大扩展，初值 300 秒；算术溢出拒绝。
- heartbeat 要求 `now < current_expiry`、时间单调、requested expiry 严格大于 current expiry。
- heartbeat 不能改变 attempt、run、effect、request、generation、owner 或 token。
- terminal/interrupt/recovery 与 heartbeat 对同一 revision 做 CAS，只有一个事务能成功。
- 过期或终态 Attempt 永远不能由 heartbeat 复活。
- 单次扩展有上限，但合法 heartbeat 可重复，因此不限制任务总时长。

Runtime 以剩余 TTL 约三分之一为周期续租，并保留足够的调度安全余量。续租拒绝或 Application
通道失败后立即关闭该 Attempt 的本地响应准入，触发对应模型、工具或 delegated cancellation；
不提交伪造的 Failed/Cancelled。租约到期后由现有 recovery 按 replay safety 决定 Retry、Resume 或
OutcomeUnknown。

## 已完成接线

1. Kernel claim/heartbeat 已增加最大扩展、严格增长、过期与溢出约束。
2. StorePort/Store 已增加单 Attempt heartbeat CAS；schema 保持 20。
3. Application 已增加 `HeartbeatEffectDelivery` 写命令、receipt UoW 与完整身份/事实校验；Service
   protocol 硬切为 18。
4. Runtime Driver 已将同步 dispatch 移入 scoped worker；Application port、heartbeat 和 publish
   保持在主线程，通过 response/one-shot ACK channel 串行。
5. 只在 `ExecutionStarted` ACK 后注册 timer，terminal ACK 后注销。默认 TTL 300 秒、cadence
   100 秒，自定义 cadence 必须小于 TTL 一半。
6. Runtime bridge 只保留 Host 返回的 request/attempt/generation/owner/token，并用最新已 ACK expiry
   提交响应。续租或响应准入失败只关闭对应 Effect 的准入、移除其 timer 并调用 scoped cooperative
   abort；同批其他 Effect 继续续租和结算，Runtime 最终返回首个准入错误。
7. CLI 已把 Runtime renewal port 接到 Application Service，并把 scoped abort hook 接到活跃 Effect
   的 `ExecutionCancellation` registry。模型、普通工具、runtime tool 与 delegated session 均注册
   Effect 级 cancellation；delegated 在 suspended 和 drain 期间保持注册，终态后由 RAII 清理。

## 验收

- 短租约真实 SQLite 测试：Host 静默超过原 expiry 后 terminal 仍以新 lease 成功结算。
- 双连接对同 revision 续租只有一个成功；row JSON、物化 expiry、revision 和 Run event 一致。
- heartbeat 与 recovery/terminal 同时释放，最终只能是合法串行结果，终态不可复活。
- heartbeat 事务或最终 receipt 故障不返回 ACK，也不改变 Host 本地 lease。
- 真实 CLI HTTP fixture 在收到请求后延迟 headers 170ms；100ms TTL、25ms cadence 下首字节前已
  续租，最终 terminal 使用最新 Application ACK expiry。
- 真实 CLI runtime tool handler 启动独立 `/bin/sh` process group 并静默超过原 TTL；拒绝第20次
  heartbeat 后目标进程树退出、完成 marker 不存在、无关 Effect 未取消，Application 只接纳
  `ExecutionStarted` 与已发生的 progress，未接纳 terminal 或伪造 Failed。
- 两 Effect 交错 fixture 中，一个续租失败只关闭并取消自身；另一个继续至少两次续租并成功提交
  terminal，最终 Runtime 返回首个 Effect 的错误。
- delegated/permission 静默等待同样续租；失败后的 recovery disposition 与 replay safety 一致。
  （待补真实长等待）

当前已通过短 TTL + 真实 Application SQLite 的 Runtime 静默/拒绝/竞态测试、真实 Service IPC
Begin→Claim→Start→Heartbeat→业务响应，以及 CLI cancellation registry 测试。Application 303/303、
Store 278/278、Protocol 28/28、Runtime Driver 10/10、Service 70/70、CLI delivery barrier 8/8，
workspace all-targets check 均通过。

该切面完成了通用 Effect 执行期间的持续所有权骨架，以及真实 HTTP 首字节和子进程取消证据。
delegated/permission 的真实长等待故障验收仍待补齐；RuntimeTask 的 transition-specific lease、常驻
Supervisor wake/backoff/lifecycle、跨 Run 公平调度和 §20.3 完整长任务验收仍是后续范围。
