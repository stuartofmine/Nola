# 当前 Run 冷恢复：本机进程观察

状态：实现及 SQLite 回归已落；跨平台最终编译、真实双专家 kill/relaunch 验收另记实际结果，不能以本页替代。

## 问题与边界

iOS 本机专家 Run 的旧 worker 已退出，但恢复只查看 effect lease 到期时间，因而仍等待原 240 秒租约。进程死亡仅证明该旧进程不会继续写，不能证明外部操作失败，更不能授权重放未知写入。本批不改变租约长度、时钟、HostReplaySafety、unknown outcome 的人工处理或原 Run 所有权。

## 唯一共享判断

- `inspect_run_process_owners {conversation_id, expected_run_id}`：同一 Core 只读快照验证本机未终结 conversation Run、assistant owner 与 frozen invocation；返回原 config digest/binding、完整 delivery leases、Core 解析并去重的 process owners、unobservable 标记及独立 session lease 是否存在。
- `validate_run_process_observations {plan, observations:[{owner, observation}]}`：同一只读快照重新读取完整 plan，逐值不一致即拒绝。必须完整且每个 owner 仅一次；未知或仍活进程返回 false。相同 PID 但不同 boot/start 可以证明旧进程终止。合法 unobservable worker 返回 false，保留原 expiry 路径。
- Host 只读取 OS，不解析 generation/owner 字符串，不自行决定恢复策略。iOS 使用既有 sysctl/kill 观察，Android foreground 使用既有 `/proc` shim，embedded child pump 使用 Rust Linux/Android OS 观察；其他平台 native shim 无证明时返回 Unknown。
- 判真后仍调用既有 `recover_observed_orphaned_run`，提交原 frozen binding、原完整 leases、真实时间与原 session lease 事实。真正写事务再次比较 attempt/generation/owner/token/**expiry**；观察后续租或换 token 必须重新观察。恢复仍由原 Core replay safety 分类生成计划。
- 独立 CLI session lease 仍保留现有保守 PID 判活与 exact lease CAS。移动端不能观察该独立 owner，存在 session lease 时不提前接管。只读 validator 证明的是 delivery owners，并不代替 session 证明或授予执行权限。

## 调用入口

- CLI `core_agent_runtime::observe_run_owners` 保留独立 session guard，复用新 Core delivery plan/validate；没有完整死亡证明明确拒绝接管，不编造活 PID。
- iOS `IOSCanonicalAgentApplication.recoverExactRun` 在等待下一租约 deadline 之前尝试共享观察；false 按原 wake 时间等待，true 经原恢复命令后重新进入原调度循环。
- Android foreground `AndroidCanonicalAgentRuntime.recoverExistingRun` 同模式；嵌入 child pump 的 recovery 也接同 plan/validate。observed reconciliation 成功不重复 expiry reconciliation，重读 frozen invocation，sealed 即退出。
- 该 Host OS 接口沿现有本机可信控制面，不向 Web renderer 添加任意 observation→write 转发。只读返回 true 不是可持久化的授权凭据。

## 验证

新增真实 SQLite 用例 `dead_worker_observation_recovers_before_expiry_and_fences_renewal`：未到期 240 秒 delivery、活/未知观察拒绝、PID 复用核 start、完整 owner 集合、错误 PID、unobservable、续租后 plan 与写 CAS 都拒绝、重新观察后提前生成原 Resume 计划。最终日志、全套计数与真模型 kill/relaunch 结果在本轮主验收报告记录。

### 本批已执行证据

- `/tmp/nola-cold-observation-delivery-suite.log`：Application + SQLite delivery 套件 **12/12 passed，0 failed，0 ignored**，含新增提前恢复用例及原 unknown outcome / rejection / rollback / heartbeat 回归。
- `/tmp/nola-cold-observation-mobile-final.log`：移动 Rust bindings `cargo check` exit 0；这不是 Android ARM/iOS 真机重建或 kill/relaunch 验收。
- CLI 使用真实隔离子进程退出的 `observed_recovery_` 回归 **2/2 passed**（`/tmp/nola-cold-observation-cli-test.log`）；额外 wrong-run 隔离回归 **1/1 passed**（`/tmp/nola-cold-observation-cli-wrong-run.log`）。这些是共享 Host/SQLite 回归，不替代移动真模型重启验收。
- inspect/validate 整体由 `with_consistent_read` 包裹；Host OS 观察在两个只读快照之间。观察后状态改变必须重新读取，真正写事务再次 fence。正常并发终结只在精确原 frozen invocation 已 sealed 时回原终态路径，不吞仍 active 或身份改变错误。
