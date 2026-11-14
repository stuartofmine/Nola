> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Provider 指令投影与缓存边界

状态：协议层整改；不代表真实 provider 缓存命中率已验收。

## 所有权和输入输出

Core 持有授权、恢复阶段和请求内容的权威状态。Host 将已提交的请求投影为 canonical messages；provider adapter 只负责线路协议，不重新决定授权、不推断用户需求、不根据工具输出提升权限。

输入包括有序 messages（role/content、工具调用 ID、结果 ID）、本 Step 工具 schema、模型路线和显式缓存配置。输出是最终请求 body；真实缓存收益以该请求的 provider usage 为准。内部缓存指纹、相等的 JSON 前缀都不是命中证明。

稳定政策和动态信息必须分开理解：

- 稳定政策：Core 定义的约束、工具权限语义、应用规范。
- 动态可信指令：当前恢复要求、权限拒绝后的动作限制。即使变化频繁，也不能为缓存而降级成 user 内容。
- 动态观察数据：工具结果、授权收据、诊断报告。它们不是新授权；其中引用的网页、文件、命令输出也不能成为系统指令。

Core 现在在生成处将权限收据、预算计数、工程错误/资源列表、Skill 待完成记录和完成审计数据显式投影为 `nola.runtime_facts.v1` JSON，使用尾部 user-role 传输；这是数据，不是用户新请求。静态政策仍用 System。最终合成的工具证据、候选答案和审计报告不再提升为 System。独立审查按 canonical 历史消息身份区分真实用户请求与传输用事实，不再只看 role。Host 的批次拒绝和协议失败也在生成处写运行事实，不通过关键字或消息 ID 猜测降权。

真正阶段控制（权限拒绝、工具预算关闭、候选/审查阶段切换）仍保持原指令级别。不同阶段的指令集合可能改变，因此并不承诺 Anthropic 在所有状态转换都保持历史缓存；常规变量更新不再改写同一政策文本。

## 各协议的确定行为

| 路线 | 指令位置 | 缓存边界及限制 |
| --- | --- | --- |
| Chat Completions | 保留 system/developer 的原始顺序；工具协议修复不把尾部恢复指令前移 | 稳定历史不因指令归并被主动改写 |
| Responses | system/developer 以有序 input 消息投影；保留 function_call/output 配对 | 不把动态指令拼进另一份顶层 instructions |
| Anthropic 当前路线 | system/developer 保留为顶层 text blocks，按原相对顺序；不降级为 user | 单独标记 leading instruction segment 末块，因此后面的动态指令变化不改变该政策块；动态顶层指令仍使其后的整个历史前缀变化 |

Anthropic 部分路线可能支持原生会话中 system 更新。参考 pi 的 `supportsMidConvoSystemMessages` 是显式能力，而不是按模型名猜测。Nola 尚未声明或验证该能力，所以真正指令保持原级别。仅拆分顶层 blocks 不能保证跨阶段缓存；本轮消除的是混入其中的可变观察数据，不能静默降低安全指令权限。

## Anthropic 缓存写入规则

1. 开启缓存时，leading instruction segment 的最后一个非空 text block 设置一个断点。这个 segment 是可缓存候选，不保证内容事实上不变。
2. message 断点落在指定消息的最后一个支持缓存的 block，而不是第一个 text。相邻 user 消息合并后，所有并行 tool_result 和图片/PDF 数据仍在断点之前。
3. 不为纯工具结果或图片消息追加虚构的空 text block。
4. 一次性 fork 的 skip-cache-write 请求只标记共享的倒数第二条消息；只有一条消息时不标记。该函数是调用方的显式策略，普通请求不自动变为 fork。
5. 不随断点前移给旧 tool_result 增补 cache_reference。缓存编辑是另一种尚未实现的能力；没有 cache_edits 事务，就不应重写旧结果形状。
6. 不改变 tool_choice 来阻止摘要工具调用。缓存配置不授予执行能力，具体调用仍由 Core 授权。

工具调用和结果 ID 不变，结果不得被夹在一对 tool_use/tool_result 之间的动态指令打断。Anthropic 会合并相邻同 role 消息，但不改变结果块顺序。

## 验证与未完成事项

自动化入口：`privileged_instruction_order_and_cache_boundaries_survive_wire_projection` 比较 Chat、Responses、Anthropic 最终 body，覆盖 developer 权限等级、动态指令更新、稳定政策断点、两工具结果配对以及不可信结果文本。`anthropic_cache_*` 覆盖多模态末块和单消息 fork。媒体/PDF、canonical tool protocol 的既有回归必须继续通过。

真实验收还需：同一模型路线连续请求的最终 body 与 cache-read/write usage；改变动态权限状态的请求；完整工具结果后热分叉压缩。不得以 LAN Chat 的收益推导 Anthropic 收益。当前未连接真实 Anthropic 服务验证，历史缓存改进也未闭环。

参考：`参考资料/pi/packages/ai/src/api/anthropic-messages.ts` 的 `convertMessages`、compat 和最后 block 缓存；`参考资料/claude-code/src/services/api/claude.ts` 的 `addCacheBreakpoints` 和缓存编辑专用字段。引用特性不意味着 Nola 已具备对应协议能力。

## 原生专家提示词模板的默认来源

专家显式保存的 `selectedPromptTemplateID` 优先；未指定时不读取父主会话的模板。客户端空值选项显示“未指定模板”，不承诺跟随主会话，也不把未指定解释为跨平台完全相同的默认模板。

- macOS 从 `MacPromptLibrary.defaultTemplateID` 读取应用全局默认；没有已选全局模板时捕获 `NoTemplate`。
- iOS 的现有捕获路径在专家 defaults 带有 `language` 时使用应用全局选中模板，或该语言的内置默认；没有显式模板且没有该字段时捕获 `NoTemplate`。新建专家通常不保存 `language`。本次仅纠正文案，没有改变这条既有选择语义。

模板选择及可读正文通过可信 Host capture 交给 Core，不能由模型参数提供模板正文。已冻结任务从原捕获记录恢复，不因全局默认后来变化而重新选择模板。
