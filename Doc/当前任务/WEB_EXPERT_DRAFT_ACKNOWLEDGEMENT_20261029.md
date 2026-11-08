# Web 专家父消息的草稿确认

状态：2026-10-29 已通过根静审与本批定向检查。

`Chat.svelte` 原来在完整专家准入及历史确认后，仅凭 `draft === input.text` 清空编辑器。等待期间清空再重输相同字节属于新草稿，原字符串判断会误清。

当前修复复用 `workspaceDrafts.ts` 的 Host／会话归属和 `editRevisions`：仅在新 expert intent 建立时捕获原文本版本，同一 submission 重试不重新采样。它不是普通 Run，未构造 `PendingSourceSubmission`、用户／助手消息 ID 或假 receipt。

历史确认后，草稿确认进入原持久化队列；实际 IndexedDB repository 仍使用既有 per-key revision CAS。只有原版本未变才请求落盘空文本，objective／attachments 保留；写入返回后再次核版本才清内存。等待写入期间发生的新编辑排在清空写入之后，保留新内存和磁盘内容。持续排空已排队写入且该 owner 无持久化错误后，页面才清除 localStorage expert intent。写失败或后续新草稿写失败均拒绝确认，保留 intent 供同业务身份恢复。

删除会话清除对应内存 ticket；Host 失效禁止旧 owner 确认。跨 Host 不复制 ticket。

**当前冷恢复限制**：ticket 仅是当前 workspace 生命周期中的编辑见证，未添加新的持久格式。冷恢复只有 durable expert intent、没有原编辑版本见证时，仍确认 canonical 历史与 intent，但保守保留当前草稿，即使字节与已发送文本相同。不会凭字符串重建旧 ticket 或误清用户输入。

回归源码覆盖：正常只清文本／原附属草稿保留、ABA+同 intent 重试、await CAS 期间同字节新编辑、clear 写失败、clear 成功后 successor 写失败、冷恢复无 ticket、Host 失效及会话删除。受控 repository 测试验证队列与失败语义；它们不冒充真实 IndexedDB CAS 或真实模型端到端验收。

实际验证：`node --test src/lib/conversationDraftRepository.test.ts src/lib/workspaceDrafts.test.ts` 共 **20 passed / 0 failed / 0 skipped**（含新增 6 case）；`npm run check` 为 **0 errors / 0 warnings**。日志分别为 `/tmp/nola-web-expert-draft-20261029-tests.log` 与 `/tmp/nola-web-expert-draft-20261029-check.log`。没有扩大到全套、模型调用或新的浏览器点击测试。
