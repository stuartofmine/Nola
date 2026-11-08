> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Host 响应提交握手与重新采样隔离（2026-09-09）

本阶段承接当前 Run 恢复迁移，核对 `ARCHITECTURE_V2.md` §3.2、§7.4、§9.2。
保留工作区全部修改；没有提交 commit 或操作用户数据库。

## 架构定位

Application 已通过仓储 Port 组织事务，bootstrap 装配 SQLite；但 Application 与
Runtime Driver 仍生产依赖旧 Core，完整状态机、事件和 Effect 装配尚未归纯 Kernel。
CLI 仍混合客户端、Web transport 与 Host；Android 计划任务仍有本地领域写入。
当前 Run 启动恢复规划已迁入 Application/Kernel，持续 Supervisor 尚未完成。

`RunAttemptId` 当前由 Host request ID 映射，`model_request_part_owners` 只是 Part
归属记录，没有独立 Attempt 账本、activeAttemptID 或完整状态转换。本阶段不以换 Part
ID 代替真实 Attempt，也不宣称 §3.2 全部满足。

## 本阶段修复

原 Host 回调返回 unit；Application 接受失败只能保存在 Runtime 的局部错误变量中，
Host 在 dispatch 返回前仍可能执行 retry/fallback 的第二次 HTTP。工具传输截断还可能
发生在 reasoning 已发布之后，原 LLM 局部 buffer 清空无法退休 Core 已接收的片段。

现 AgentHostDriver 的响应回调返回 `Result<(), String>`，Host 必须传播拒绝，只有接受
后才递增序列。Runtime 保留 Application 错误优先级，拒绝后不发布结果或调度后继。
LLM `stream_chat_checked` 把事件拒绝与 provider 错误分类分开；即使拒绝消息包含 503
或 connection reset，也不能触发 retry/fallback。原无失败回调入口只作为便利包装。

模型 transport retry、provider fallback 与协议纠正的重新采样先提交
`ModelStreamRestarted`，成功后才清空缓冲、换 Part ID、请求 provider。
普通 Application execute 也把 restart 当作立即持久化边界，不能仅由热状态确认成功。
Compaction 的重新请求同样使用可拒绝的回调握手。

## 验证

最终整合均通过，测试进程已结束：

- Application 279/279：`/tmp/nola-host-acceptance-application.log`。
- Runtime Driver 3/3：`/tmp/nola-host-acceptance-driver.log`，包含提交拒绝后的错误归属、
  不发布、checkpoint 不变和随后正常执行。
- CLI Host 93/93：`/tmp/nola-host-acceptance-cli-verified.log`。
- checked LLM 真实 HTTP 3/3：`/tmp/nola-host-acceptance-checked-verified.log`，覆盖 delta、
  retry、fallback 拒绝，拒绝文本故意包含可重试错误词。
- 真实 Host→进程内 Application 2/2：`/tmp/nola-host-acceptance-provider-verified.log`。
  已发布 reasoning 加工具碎片后截断，成功重试后的规范 Part 和冷 checkpoint 无旧内容；
  SQLite inbox 写入故障拒绝 restart，第二次 HTTP 不发生，规范表不变。
- `cargo check --workspace --all-targets` 退出 0：
  `/tmp/nola-host-acceptance-check-verified.log`。

子任务环境首次真实 HTTP/进程采样测试因 EPERM 未执行完整断言；root 在当前授权环境
重跑上述实际用例全部通过，没有跳过或弱化断言。真实 HTTP fixture 使用本地 provider
stub，不能作为真实模型或 Service IPC 验收。本阶段没有重跑平台原生/UI 验收。

## 下一步

在该拒绝握手之上建立规范 Run/Attempt 账本和准入事务：区分逻辑模型调用、物理采样、
Effect 投递与 Core Turn；新采样创建新 Attempt，单纯重投递不冒充新采样。
继续收敛 Supervisor/lease、旧 Core 所有权、CLI/Android 领域边界，以及 §20 五端、
真实模型、取消/重启和 10M UI/性能验收。
