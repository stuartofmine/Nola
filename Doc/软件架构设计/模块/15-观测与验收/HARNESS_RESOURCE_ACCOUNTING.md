> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Harness 资源归属与晚到观察契约

状态：2026-10-12 的目标契约，尚未完整接线。Shell 最终观察原子提交已实现；不能据此
推导资源归属、Agent 晚到观察或累计账本已经完成。本文件替代
`../05-执行与恢复/HARNESS_CONTROL_SCHEDULING.md` 中“每个新 attempt 必须建立新物理计量作用域”的含义。

## 审计结论与模块边界

当前 `execution.rs::run_agent_task` 在运行入口创建一个 `resource_usage`，watchdog
直到该 worker 退出才停止。`RuntimeTaskExecutionContext::prepare_permission_resume_with`
却明确要求恢复授权后返回新 attempt、新 generation、新 token。最后读取
`execution.active_lease()` 再写整份 usage，无法证明计数只属于这个新 attempt。
此外 `current_process_tree_usage` 包含宿主，`ps` 只看到当前存活进程：这个数字既不一定
独占，也不是完整生命周期累计。最后 ACK 丢失、归属错误和采样缺失是三个不同问题。

| 模块 | 唯一职责 | 禁止行为 |
|---|---|---|
| OS meter adapter | 报告来源、物理身份、计数语义及可观测范围 | 把查询失败补零；把活进程快照称为累计消耗 |
| Host supervisor | 建立已准入的物理 scope、观察生产者和传输队列 | 从当前 attempt 推断 scope 归属；自算权威 Task 总量 |
| Domain | 纯数值与转换规则、错误分类 | 读取 OS、数据库或推断执行授权 |
| Application | 校验 scope 准入及观察身份，准备唯一累计变更 | 因接受晚到观察而重新 claim 或续租 |
| Store | scope、binding、观察、水位、聚合的原子提交与重放 | 修改已封存业务 receipt；允许第二套累计来源 |
| Client | 展示已提交聚合和覆盖程度 | 把缺样本显示为 0%；从局部事件自行补总数 |

执行权限与计量权限是两种能力。执行 lease 被撤销，不表示已发生的物理消耗不存在；
持有旧观察证据，也不表示还能启动工具或更新新 attempt。

## 身份与数据模型

| 身份 | 生命周期 | 必填证据 |
|---|---|---|
| Task / Run | 业务任务及本次语义执行 | 正式 Task→Run binding，不从会话当前运行推断 |
| attempt | 一次执行授权 | attempt ID、generation、lease；权限恢复可能更换 |
| meter scope | 一个物理累计计数器的连续生命期 | scope ID、所属 Task、adapter、OS job/出生身份、计数语义、独占性 |
| producer incarnation | 一次观察生产者生命期 | 已准入 producer ID 与 epoch；不可用 PID 代替 |
| scope binding segment | scope 对执行阶段的归属区间 | 原/新 attempt、边界水位证据或明确 unavailable |
| observation | 不可变测量事实 | scope、producer、sequence、observed-at、payload、coverage、payload digest |

同一 worker 等待权限后继续使用同一 OS job：scope 不变，新增 binding segment；不得
重置物理计数器水位，也不得把整份累计计数重复写入新 attempt。若恢复启动了新 OS job，
才新建 scope。scope 禁止跨 Task 隐式转移，跨 Task 复用宿主必须建立独占子 job 或声明
无法归属。单纯分配 UUID、记录当前 PID 或重新读一遍计数，不构成新计数器的证据。

每个指标单独携带 coverage：`complete`、`lower_bound` 或 `unavailable(reason)`；不能
用一份 overall coverage 掩盖 CPU 未知、内存可读的差别。真实零是带来源的数值；未知
没有伪造数值。固定原因至少包括 `query_failed`、`scope_not_exclusive`、
`counter_discontinuity`、`boundary_not_observed`、`final_counter_unavailable`。
诊断原文是可选附件，不参与错误分类或执行授权。

CPU 来源必须区分：

- `lifetime_cumulative`：同一物理计数器的累计量，平台保证包含其已退出子进程时才能
  声称完整。首次观察必须关联 scope 创建基线或 OS 明确的零起点。
- `live_process_snapshot`：当前存活进程的累加，仅保存观察证据。不得当作前一种来源
  做连续差分；进程退出会改变集合，两个快照差不能代表该区间 CPU 消耗。

内存/进程峰值也区分 OS lifetime peak 与采样 peak。非重叠 scope 有证据时才可对峰值
取 max。并发 scope 的 Task 峰值是“同一时点各独占 scope 用量之和”的最大值，不能
取各自峰值的 max，也不能把各自峰值之和声称为实际峰值。只有各 scope 峰值时，前者
至多是下界，后者至多是上界；采样不完整则连完整上界也无法保证。没有共同单调时钟
及完整 scope 集合，不能按相近墙钟时间拼出完整峰值。输出字节有独立来源：Shell 输出文件水位或
canonical journal 的已提交游标；不能拿 worker 保留的截断预览长度代替正式输出量。

## 写入与计算

1. scope 准入保存平台能力、Task 归属、物理身份和计数器语义。数据库 intent 与 OS
   创建不可能合为一个事务：先记录 intent，OS 返回身份后绑定；ACK 不明时核对原
   intent/job，不创建第二个 job。只有完成该准入才能把其观察计入 Task。
2. Host 在发送前固定 observation 的完整载荷、原 sequence 和 digest。重试不换时间
   或 ID。Store 对 `(scope, producer epoch, sequence)` 建立唯一约束。
3. 同键同载荷返回原 ACK；同键异载荷为 `observation_conflict`。乱序新样本可保存为
   历史证据，但不能回退当前水位。接收时间不替代观察时间，计数器回退必须显式报错。
4. 只有归属、来源和基线均有效的累计计数可计算 `new - committed`，用 checked
   arithmetic 加入 Task 总量。水位键是物理 scope，不是当前 attempt。不同 producer
   接手同一 scope 也不能各自从零累计一次。
5. attempt 分摊使用 binding segment 的边界水位。权限边界没有观察时，保留
   “Task 可归属、attempt 区间不可拆分”；不得平均分配或全部塞进恢复后的 attempt。
   Task 总量不依赖所有 attempt 分摊已经完成。
6. 观察、去重事实、scope 水位、segment 更新、Task 聚合与预算事件同一事务提交；
   CAS/存储故障全部回滚。ACK 返回提交游标、scope 水位和 coverage，不仅返回 bool。
   只读查询和分页历史读取均绑定该游标，客户端不通过拼接最新行制造混合快照。

不得同时保留“旧 ReportProgress 整体覆盖 usage”和“新 scope ledger 累计”作为两条
权威写路径。接线批次必须将预算读取、终态观察、恢复和客户端投影一起切换到 ledger；
旧 usage 字段只能成为同事务生成的读投影，不能继续接受 Host 提交的 Task 总量。
未上线不意味着可以删除开发者的会话数据；存储格式变更需明确本地数据处理步骤，
不得偷偷删库或将不可解释的旧计量补零。没有隐式版本猜测或多套接口长期并存。

## 执行流与观察流

```text
执行： running → waiting_permission → 新 attempt running → Run sealed
计量： scope open ──同一物理 scope / 新 binding segment──→ closing → settled
                                      │                           │
                              边界缺样本：不分摊            最终计数未知：gap
                                                                  │
                                                 incomplete → 原身份证据补齐
```

Run sealed 不是 OS scope 已关闭的证据；scope settled 也不是任务成功的证据。关闭 scope
要求宿主明确处理其进程/资源的停止确认，未确认则保持 closing，不伪造 final sample。

| 情形 | 允许的状态变化 | 不允许的变化 |
|---|---|---|
| 正常采样 | 原 scope 水位、聚合及 coverage 前进 | 续租或业务终态变化 |
| 等待授权 | 停止新执行准入；保留原 scope 与等待开销 | 自动结束 scope 或把等待伪装成失败 |
| 权限恢复 | 新 attempt binding；原 scope 水位延续 | 重计整个累计量 |
| OS 查询失败 | 保存 unavailable，保留最后有效水位与缺口 | 写零、伪造预算耗尽 |
| 业务封存后晚到 | 补原 scope 观察及计量结算状态 | 改不可变业务 receipt、重跑模型 |
| 接管后的旧生产者 | 仅受限的原区间补证据 | 更新新 producer 水位或获得执行权限 |
| 提交 ACK 不明 | 查原 command/observation receipt，重放同载荷 | 新 ID 再计费或重新执行副作用 |
| 数据库不可写 | 保留待提交观察，显示未确认 | 将“尝试写入”当成持久成功 |

晚到观察需要**预先准入的观察权限**，不能凭一个历史 lease 任意修改 ledger。其边界
包含原 scope/producer 身份及已发放的序列区间；接管/关闭原子记录旧 producer 的截止
范围。截止范围内晚到只补原 scope，超范围拒绝；同键重放仍返回原 ACK。若崩溃前无法
确定旧 producer 的最终范围，保留 gap，由正式 OS final evidence 恢复命令解决，
不能要求旧进程重新执行来“补齐”。

计量关闭状态使用独立 `open | closing | settled | incomplete`，不得复用业务的
`completed | failed | cancelled`。晚到证据可使 incomplete→settled，但不改变业务
结论。已观测超限保留为资源事实；未知不等于超限，低于阈值的下界也不证明未超限。
硬限制需要平台 enforcement 准入；不支持时明确拒绝或由用户改变策略，不能静默降级。

## 必须通过的纵向验收

| 场景 | 必须检查的持久证据 |
|---|---|
| 同 job 跨权限恢复 | 两个 attempt、一个 scope、多个 binding；CPU 只计一次 |
| 恢复另启 job | 两个 scope；旧 scope 的未知尾部仍存在，不归零 |
| 边界漏采样 | Task 可累计，但 attempt 分摊不可判定，不强行分配 |
| 同样本重复/异载荷/乱序 | 原 ACK、明确冲突、当前水位不回退；聚合不重复 |
| 旧 producer 与接管竞争 | 同事务截止范围生效，新生产者状态不被覆盖 |
| seal 与 final observation 两种提交顺序 | 相同业务 receipt；资源事实最终一致，不新增模型调用 |
| 事务中断/ACK 丢失/重开 | 全部回滚或原 ACK；scope、聚合和缺口一致 |
| 进程退出/PID 复用/共享宿主 | 不混入外部任务；无法归属保留 unknown |
| 两个并发 scope 的峰值不同步 | 不把峰值之和当成实际峰值，不把单 scope 最大值当作完整 Task 峰值 |
| UI 切会话及冷启动 | 读取该 Task 的同一已提交投影，coverage 与累计值不串会话 |

性能先采真实基线：OS 采样时长与 CPU 开销、观察事务时长、每 Task ledger 大小、恢复
读取行数、晚到补齐延迟；按平台及 source 分组，保留失败和缺样本。不能把 watchdog
100ms 间隔当作采样成本或 UI SLA。高频样本可合并传输，但必须保留累计水位、峰值、
coverage 变化与终结证据；保留/归档策略须证明重放去重仍成立，不能直接删除唯一键。

当前实现差距：上述 scope 准入、ledger、binding 分段、晚到权限、持久待提交观察与
多端读投影均待实现。当前 ps 适配器只能提供活进程快照；不得先将它套进累计 DTO，
再用单元测试的递增数字宣称真实资源计量通过。
