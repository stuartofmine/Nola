> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# V2 上下文超限恢复与摘要来源定位：2026-09-10

本轮按 `ARCHITECTURE_V2.md` 的 Context Aggregate、单一消息真相和可恢复 Run 约束，
初步收敛 provider 上下文超限后的恢复链。保留当前工作区和用户数据库。

## 已实现

- Core 的待压缩计划除来源 SHA-256 外，现持久化被选择条目的稳定 `ItemId` 列表与压缩前
  token 估算。压缩完成后，这些字段进入规范 transcript 中的 `AgentItem::Compaction`；
  Conversation Projection 的 Compaction Part 返回同一组原文 ID 和实际 token 统计。
  原文仍保留在无损 transcript，摘要不再是无法回指来源的孤立文本。
- 来源身份、指纹、选段和保留 suffix 都在 Compact Effect 发出前进入 Core checkpoint。
  因此进程退出后恢复的是同一份选择，Host 不能用另一段历史替换其来源。
- CLI Host 扩充 provider 超限归类，覆盖 `context_window_exceeded`、`prompt is too long`、
  `request_too_large`、超过最大 token 数及要求缩短 messages/prompt 等常见响应。
  这些错误不会在同一物理请求内做普通网络重试，而是提交 typed
  `context_length_exceeded` 给 Core，由 Core 的每 Turn 有界压缩计数负责恢复。
- Application 冷恢复回归验证：原 Model Attempt 失败后，压缩结果继续原 Run 和原 Turn，
  创建新的 Model Effect/Attempt；用户输入在 transcript 中仍只有一份；receipt 冷重放不再
  压缩一次；摘要的每个来源 ID 都能定位到压缩前的规范 transcript 条目。

## 验证

- `cargo test -p nola_core --test compaction_retry_checkpoint`
- `cargo test -p nola_application --test compaction_kernel_receipt_e2e`
- `cargo test --manifest-path cli/Cargo.toml provider_context_overflow_variants_enter_typed_compaction_recovery`
- `cd core && cargo check --workspace --all-targets`

## 当前边界

本轮覆盖直接模型请求的 provider 上下文超限以及冷恢复。若 Compact 请求自身因为单个不可分割
的大条目超过 provider 窗口，当前仍以 CompactionFailed 终止；后续应引入 Artifact/Blob 引用
或分层摘要计划，不能靠无限缩小目标值或重复发送同一超大正文。

历史数据库中已存在且没有来源字段的旧 Compaction Part 只能保持“来源未知”；V2 硬切换后新
生成的压缩条目必须携带来源。完整跨端原文详情交互和千万字符性能验收仍按主架构 §20 执行。
