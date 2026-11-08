> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Idle revert：规范边界与有界上下文分离

2026-09-08。本轮保留工作区既有变更。

`Application::revert_conversation` 不再两次完整加载会话正文。新增 StorePort `ConversationRevertRepository` 与 Store metadata查询，仅按可见Message的ID/ordinal取得会话revision、真实boundary ordinal、保留parent和待隐藏Message IDs。

无checkpoint或Core history为空时，调用共享 `canonical_branch::restore_canonical_context_before`，将原来新turn的相同32条/UTF-8预算选择锚定到真实boundary（before ordinal = boundary ordinal + 1）。boundary原本不在最新tail窗口也合法；没有伪造Core item ID，也未新增Core命令。再沿用既有Core `RevertConversation`及`commit_checkpoint_projection`。非空既有checkpoint仍保持原有Core边界规则。

context excerpt notice改为归属所选第一条Message：原先归属最后一条、却插在items[0]，会让keep_message=false把该notice当成boundary最早item而误删全部选中上下文。

事务仍由既有durable receipt外层UoW保证。新增 `core/nola-application/tests/revert_bounded_receipt_e2e.rs`使用80条Message和10MB原文，覆盖tail窗口之外的boundary、keep两种模式、receipt最终写失败时visibility/checkpoint/outbox/journal回滚、冷重启精确重放、隐藏boundary拒绝与非空checkpoint二次revert。

**边界**：正文/context分配有界，但大量待隐藏Message IDs及projection mutations仍为O(N)元数据。下一步需set-based visibility写入/分页journal契约；本次不能声称整个revert输入输出及所有元数据都恒定内存。共享 context 入口现已消费共同 Historical adapter，并对最终 AgentItems、ownership 与说明执行合计 32768 bytes 预算；Application revert 专项已通过。
