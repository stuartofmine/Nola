> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 规范历史上下文的来源与引用映射

2026-09-08。实际实现为 `core/nola-application/src/historical_context_references.rs`，由
`lib.rs::durable_conversation_items` 调用；不是仅供测试的解析器。

## 共同读取与两阶段映射

选中的 Message/Part 只经过一次共同 Historical reader，所有已加载语义共享 32768 bytes
编码预算；原文缺失、字段 Deferred、整 Part NotLoaded 与 Unsupported 保持不同事实。
随后用这些验证过的投影建立引用索引，再生成 AgentItems，因此 Citation 出现在 Source/Tool
之前也能解析。没有重新读取整个 JSON、猜测 preview 参数或改写任何原文字节。

索引只接纳 Supported 且 binding 为 Canonical 或 VerifiedLocalDerivation 的数据。
工具必须包含完整输入和非 null 结果，且为 completed/failed；不生成 pending/running 调用。
两个原 schema 的 optional result null 不足以证明结果存在，因此仍显式省略。

Tool call、Source 和 Citation 的输出身份由目标 conversation/message/Part 加原 ID 确定性散列。
两代本地 branch 各自获得目标身份；源会话删除后从原 proof 验证，重开不改变映射。
调用结果复用同一个 call ID；Citation 指向同索引中已完整恢复的 call 与 Source，
并额外要求引用 URI 与 Source URI 相同。没有从 URI 取得权限或创建旧 Run。

## 引用范围与失败语义

查找范围是同一目标 Message、schema 和已验证原 message/turn。
这是实际支持边界：当前不跨 Message 解析引用，也不混用 AgentPart 与 MessagePartView 身份域。
这样最终上下文按完整 Message 组舍弃时，引用与工具对一起保留或一起移除；
不能在已有预算策略下先跨 Message 关联，再留下指向已舍弃工具的引用。
跨 Message 引用仍明确 unsupported，需要后续依赖组预算契约，不能声称全部引用迁移完成。

相同范围内多个 Part 声明相同 call/source ID 会使该键歧义，不按首次/末次猜测。
缺失、歧义、URI 不匹配或未完成的关联贡献有界说明；不会生成可执行半结构。
外来 turn、未验证派生来源、Deferred 或 NotLoaded 的 Part 不加入映射。

## 模型表示

Source/Citation 生成 assistant 角色的 JSON 数据记录，包含映射后的 source/call/citation 身份、
标题、URI 和已经验证且完整的 evidence/location/range 等字段。外部标题或来源内容不升级成系统指令。
Attachment/Artifact/Task/Step 也贡献有类型的历史数据记录，不恢复可执行 Attachment 或任务控制对象，
不将原 task/attachment ID 当成目标 canonical 身份。原文 URI 仍只是数据。

最终 AgentItems、ownership 与省略说明合计限制为 32768 bytes；舍弃完整 Message 组，
不会拆开 ToolCall/ToolResult。该上限是编码字节预算，并非精确模型 token 预算。

## 验证证据

所有上下文条目（包括工具call/result、原Part文本、fallback及预算/选择说明）均使用
完整目标owner与角色的独立context-item身份域；原始Part ID不会与生成条目别名碰撞。
生成时检查重复ID，预算裁剪前检查唯一ID及ownership完整性，禁止覆盖归属后再猜测组边界。

新增真实 Store → Application 上下文回归共 6 项：

- 两种 schema 的前向 Source/Citation/Tool 引用。
- 缺失/重复 Source 或 call、Deferred input、URI 不符、外来 turn 的拒绝矩阵。
- 两代 branch、删除 source、重开数据库、映射稳定且原 JSON 逐字段不变。
- 四类附件/任务历史仅产生描述性数据。
- 64 个结构化 Part 的共享预算耗尽保留 NotLoaded 说明，最终预算内工具对完整。
- 旧工具tool与新规范Part tool:call碰撞复现：最终裁剪只保留新Message，无孤立工具调用。

整合 Application lib、branch 和 revert 测试日志：
`/tmp/nola-context-identity-final.log`。
真实 Service 的 64-Part 初窗→独立 Part query→冷重启、错 owner/revision 拒绝且无 receipt：
`/tmp/nola-history-budget-service.log`。这些是本阶段专项证据，不代替五端实际交互和真实模型验收。
