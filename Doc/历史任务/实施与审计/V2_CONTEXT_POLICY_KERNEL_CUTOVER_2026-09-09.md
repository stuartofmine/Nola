> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 上下文与模型响应决策迁入 Kernel（2026-09-09）

本轮沿旧 Core 真实调用继续迁移，保留工作区修改、公开 API 路径与 checkpoint JSON。

- ContextWindowConfig 的默认值唯一归 Domain facts；归一化、prompt 上限、自动压缩阈值、
  目标预算及 provider 窗口更新规则由 Kernel 执行。Core 保留既有 serde 结构与薄适配。
- UsageOrigin、ContextUsageSnapshot 归 Domain。Kernel 接管当前估算/测量值、累计 token
  数字账本、normalized total 和用量投影；Core 提供每条 AgentItem 的估算值并应用结果。
  provider metadata 保留原类型和覆盖行为，未混入 numeric facts。求和使用饱和运算。
- PendingCompactionPlan 摘要空白/Unicode 长度预算校验转入 Kernel，沿用原错误语义。
- 模型规划中的 Goal 状态呈现、最终综合回答和暂缓文本呈现三个决策转入 Kernel。
  输入仅为 typed origin 与实际运行事实，不从用户文字推导路由。Skill 完成步骤尚未结束时，
  模型不能凭过早的 final answer 结束整个 Turn；明确用户拒绝仍遵循原策略。

上述算法已从 Core 删除，真实方法调用 Kernel。Domain/Kernel 未新增 Core、Host、Store
或 IO 依赖。AgentItem 的具体估算、provider metadata 类型、完整 turn/model 状态转换、
工具曝光与事件/Effect 装配仍在旧 Core，不能宣称完整纯 Kernel 目标已经实现。

验证包含 Kernel 全量 65 项、Core 原模型规划 11 项、完整 Host 70 项、配置 checkpoint
恢复专项，以及新真实两轮模型响应与冷恢复的 accounting 测试。新测试验证第一次输入
8000 tokens 不与累计 cached 数字混淆；rebase 后当前上下文来自新历史，第二次 input=0
响应仍按规范上下文估算，累计 cached 可以增加到16000而不覆盖当前用量。
最终整合结果与日志见交接文档；本阶段没有新增原生 UI 或真实模型验收。
