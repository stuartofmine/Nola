> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 规范模型 Attempt 与采样准入（2026-09-09）

承接 [Host 提交边界](V2_HOST_ACCEPTANCE_BOUNDARY_2026-09-09.md)，落实
ARCHITECTURE_V2 §3.2、§7.4、§9.1–9.2 的模型执行身份与事务边界。
保留当前工作区修改，没有 commit、用户数据库转换或用户数据清理。

## 当前生产链路

Domain 定义 ModelAttemptRecord、状态和 CAS mutation；Kernel 负责准备、启动、
重新采样与终态规划。Application 在既有 Begin／CommitTransition 原事务中应用规划，
同 checkpoint、规范消息、Part、outbox、事件和 receipt 一起提交。
Store 只保存事实、校验不可变身份与 revision，不根据 Host JSON 推断重试业务。

初始 Begin 创建独立 UUID 的 Prepared Attempt，关联 Run、逻辑模型 Effect 和原 Part。
Host 在展开实际 provider 候选配置后，每次物理请求前发送 ModelAttemptStarted；只有
Application 提交 Running 状态后才能请求 provider。provider 记录协议族和端点摘要，
model 记录实际候选值，不把主模型名称套在 fallback 上，也不记录端点凭据。

重试、fallback、协议纠正和已启动请求的冷重新采样原子封闭旧 Attempt、创建新 Attempt，
切换 Run.active_attempt_id 并退休旧 partial。新 Part 绑定新 Attempt。初次采样不退休
同一 Turn 中此前已完成逻辑调用的内容。完成、失败及取消产生明确终态；终态不能复活。
每个模型 Effect 当前对应一个逻辑 invocation，重复投递不凭 request ID 新建 Attempt。

请求的 attempt_started 来自账本，不能根据 cursor 推断：生命周期 hook 也能推进 cursor。
Supervisor 只在核验投递身份后推进热 cursor，不能用整份 Core 请求覆盖账本字段。
恢复在同一事务核对 active Attempt、当前链尾、pending effect、checkpoint 和 outbox；
无 pending effect 的悬空 active Attempt 必须拒绝，不能按旧 Turn 终态结算。

模型响应必须携带实际 Attempt ID；迟到旧 Attempt 的 delta／完成不能进入当前 Part。
投影身份与 Part owner 改从账本读取，删除 request ID 充当 Attempt 的映射及旧
model_request_part_owners 表。ACK 清理 outbox 不删除 Attempt 历史。

## 生产问题与契约迁移

- 后继 Core Turn 在首次采样时尚无 assistant Message，状态投影查询失败。现在原事务
  即创建对应 Turn 的占位消息，不回退写入初始 Run 的 assistant。
- 首次 Service 热查询把已提交 started=true 覆盖为旧 Core 值，真实 IPC 测试定位后改为
  只叠加 cursor；冷查询通过不能替代这一热路径验证。
- 旧 ModelStreamRestarted Host wire variant 已删除，Core 内部退休 partial 命令保留。
- Store schema 升至 16，Service 协议升至 13；客户端从共享版本函数获取版本。
  不为旧库或旧进程伪造 Attempt，不保留旧响应协议旁路。

## 验证

最终已执行的整合证据：

| 范围 | 结果 | 日志 |
| --- | --- | --- |
| Application 全量 | 281/281 | `/tmp/nola-attempt-application-verified.log` |
| 追加准入 receipt 最终写入回滚与身份专项 | 2/2（其中1项新增，1项与全量重叠） | `/tmp/nola-attempt-receipt-final.log` |
| Kernel 全量 | 81/81 | `/tmp/nola-attempt-kernel-verified.log` |
| Core Host 契约 | 70/70 | `/tmp/nola-attempt-core-host-verified.log` |
| Runtime Driver | 3/3 | `/tmp/nola-attempt-runtime-driver.log` |
| CLI Host | 93/93 | `/tmp/nola-attempt-host-root.log` |
| LLM 实际 HTTP 流 | 11/11 | `/tmp/nola-attempt-stream-root-verified.log` |
| Host→Application 重试与准入拒绝 | 3/3 | `/tmp/nola-attempt-provider-final.log` |
| Host→Application 跨 target/model fallback | 1/1 | `/tmp/nola-attempt-fallback-final.log` |
| 真实 Service 两次重启与恢复 | 1/1 | `/tmp/nola-service-attempt-recovery-verified.log` |
| Host 协议（含删除旧 reset wire） | 11/11 | `/tmp/nola-attempt-host-protocol-verified.log` |

Store 全套 255/255 已通过 `/tmp/nola-attempt-store-suite.log`；随后新增 active Attempt
恢复事实后，Attempt 专项 4/4、恢复专项 5/5 再次通过。非 SQLite 实际 Run Port 4/4
通过 `/tmp/nola-attempt-memory-delivery.log`。全目标编译最终日志
`/tmp/nola-attempt-workspace-complete.log`。以上专项有重叠，不应相加当作独立验收总数。

新增 receipt 故障 trigger 确认 Attempt 已在同一事务变为 Running 后才拒绝最终 receipt 写入；
所有规范记录回滚，同 request ID 重试成功，冷 exact replay 不增加 Attempt 或 revision。

具体覆盖：

- 真实 Host→进程内 Application：初始准入故障零采样、重试准入故障不发第二请求、
  已发布旧 reasoning 的退休与规范／冷状态隔离。
- 新 Application 回归：重新采样并冷恢复后旧 Attempt delta 被拒，所有记录不变，
  当前 Attempt 仍可使用同一待接受序号完成。
- Store 测试：CAS、错误 Run/effect、同 Run active 竞争、outbox 故障全回滚、
  初始事件序号、链尾查询、历史 Part owner、ACK 后历史保留和恢复原始事实。
- 非 SQLite 仓储实际保存 mutation 与投递身份，不用默认空结果绕过账本。
- 既有权限、压缩、后继 Turn、热 delta、receipt 和冷重放 fixtures 均通过正式采样准入，
  不降低生产 Prepared 输出拒绝或时间校验。

真实 HTTP 使用本地 provider stub，不是真实 LLM；Service 用真实进程与 IPC，不直接
seed 规范记录。平台原生/UI 与 §20 全量验收不包含在上述证据中。

## 尚未完成的完整目标

本次账本接入直接 ModelStream 与 Compact 采样。Delegated backend 的 invocation 与内部
隐藏模型重试还需明确分层接入，不能把不可观察的内部采样宣称为已覆盖。
持续 Supervisor、完整 lease/heartbeat/过期与未知副作用恢复、配置和 Device 重绑定、
完整 Core 状态／事件／Effect 所有权、CLI/Android 领域边界以及 §20 五端、真实模型、
长任务与 10M UI/性能验收仍未完成。完整重构目标继续保持进行中。
