> **单版本开发约束：仅保留当前接口和数据格式；不做兼容、迁移或双读双写；不自动删除、重建或改写用户数据。当前版本执行恢复不属于历史兼容。**

# 终端会话：执行宿主拥有进程，Core 拥有授权

状态：PTY输入链路已接通，真实模型单任务续输及持久化回执验收通过；真实取消与另一CLI进程恢复已取消任务查询通过，未重建任务/attempt。运行中崩溃恢复等验收尚未完成。依据工具能力整改报告。

## 边界

shell worker 通过 RuntimeTaskPort 取得执行所有权；非交互后端关闭 stdin，显式 PTY 后端由同一个 worker 持有双向 IO。不得另建进程注册表，也不让客户端直接持有 PID 或终端文件描述符。

分层：Core 生成冻结的进程操作计划 → RuntimeTaskPort 准入与会话身份验证 → shell worker 持有 PTY、进程组及输出游标 → 规范任务事件 → 客户端投影。新工具不能绕过已有 permission_resolution 或执行证据回执。

权限定位器为 `ProcessHandle`，不是 `PlatformResource` 或shell命令文本。Host验证任务/会话/执行Host后返回 `ProcessInvocation`，其command字段绑定准确task_id，invocation_digest绑定包括输入、attempt和generation的规范参数；Core拒绝句柄、摘要或资源类型替换。任务排队回执不提供活跃attempt证明，续输前须读取当前状态。

## 接口

- 启动：沿用 bash 的准入入口，显式 terminal 配置选择 PTY；非交互命令保持管道后端，不偷偷升级终端模式。
- 排队回执：交互任务返回`terminal_session.state=requires_observation`与绑定原task_id的`bash_background_status`后续调用。排队和starting均不伪造attempt/generation，也不声明输入就绪；状态读取后按真实身份续输，缺身份时等待而非猜测或重启。
- 续输：task_id、预期执行代际、幂等请求 ID、有限字节输入；Core 根据真实终端拥有者与操作授权，worker 校验活跃代际后写入。
- 读取：task_id、输出 offset、最大字节数，返回 next_offset、终态及遗漏标记；等待不得持有输入锁。
- resize：有效行列范围，只发送到该终端拥有的 PTY；不控制用户真实终端或外部设备。
- 停止：使用现有 supervisor 的进程组终止，不按复用 PID 发送信号。

## 关键不变量

1. 同一 task 的续输仅作用于一次执行代际；冷恢复缺少活句柄时报告不可续输，不重启命令。
2. 输入属于执行副作用，不因 task_id 可见而授权。一次传输未知时不自动重发，防止重复执行终端命令。
3. 输入、输出均有上限/背压；终端输出持续存档、模型分页读取，不无限积压内存。
4. PTY 创建失败、平台不支持时返回明确错误，不静默改为非交互管道。
5. shell 子进程取得控制终端，父进程不改变自己的 stdin/stdout、session 或进程组。
6. 输入写入、输出读取及 resize 的失败不冒充进程退出。进程终态由 supervisor/执行证据确定。
7. TTY 控制字符仅表示输入；UI 不允许未经筛选的终端控制序列成为界面控制通道。

## 验收（最终集中运行）

隔离命令覆盖：isatty、读一行再回显、Unicode 多字节输入、resize、输出背压、输出结束、取消进程组、错误代际/跨会话拒绝、重复输入 ID、恢复后不重放。只使用临时任务和本机伪终端，不枚举或连接物理设备。

### 崩溃恢复的准确含义与证据边界

当前实现入口为 `cli/src/task_runtime_parts/supervision.rs::recover_orphaned_shell_tasks`。恢复不等于重新执行：

- 仅客户端退出、原 worker 仍有有效租约：保留原 task/attempt，不能因重新打开会话启动第二个命令。
- worker 租约过期且存在精确绑定的 Prepared 未派发证明：由 Store 原子判定是否允许重新派发；PID 不存在不是未执行证明。
- 已派发后租约过期或缺少未执行证明：记录 `reconciliation_required`，保留输出与副作用待核对；不重放命令，不以数值 PID 杀进程，也不声称物理进程已停止。
- 旧输入的送达结果不明：保留未知结果，不将输入重新投递到替代 attempt；取消后的冷查询不能替代此分支验收。

待补的进程级验收须在隔离数据库/工作区创建带唯一启动标记的 PTY，分别中断客户端与其确切拥有的 worker。检查原 task/attempt、启动标记计数、输入回执、保留输出及恢复事件；只清理测试自身仍持有的进程对象，不根据用户数据库 PID 发送信号。模拟租约到期只允许修改隔离夹具，且必须标明是故障注入，不计为真实超时等待。上述分支尚不能用取消后查询样本宣称全部通过。
