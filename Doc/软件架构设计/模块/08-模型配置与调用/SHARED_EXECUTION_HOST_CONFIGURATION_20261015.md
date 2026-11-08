> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 会话执行位置的原子配置契约

## 边界与输入输出

Mac `MacExecutionHost` 的现有 raw 值为 `local`、`remote`（NolaMac/Models/MacConversationModels.swift）。这是独立执行位置选择，不等于模型 provider，也不等于移动端的无凭据 `executionHostBinding` URL。保留现有语义，不由 Core 根据模型名称或身份推断更改。

唯一配置命令 `patch_conversation_configuration` 接受 `executionHostRawValue: local | remote | null`；省略保留，null 删除显式值。允许与 modelSelectionID、modelOverrides 同一 expectedRevision CAS 提交。Store 在同事务更新配置和 revision，回执与 Web GET projection 返回该字段。metadata 不再写此执行配置字段，避免绕过配置验证。

完整创建/settings 校验接受同样的值；现有 full settings API 本批不整体删除。Mac 负责显式模型/Host 操作使用配置命令，普通 metadata 保存排除身份、执行位置、模型参数。本批不扩展 Host 网络发现、不自动重写模型路由、不引入兼容字段。

## 整包保存的路由保护

Store `update_conversation_settings` 与 `update_conversation_profile` native 分支在同一写事务比较现有和传入的 modelSelectionID、executionHostRawValue、executionHostBinding。missing/null 同为 absence；任何不同拒绝写入并要求配置命令。创建仍可一次初始化完整目标；无关保存不重查已失效 profile，因此仍可编辑其元数据。并发目标变更会导致旧整包保存显式失败，不会覆盖新目标，客户端应只重放自己的元数据字段。

Web owner profile 的既有显式 modelSelectionID 路径暂保留，其只合并明确拥有的字段且仍执行profile可用性初始化；不是native整包替换。generation/modelOverrides的full更新本批仍保留现有用途，未封闭参数及所有metadata的并发覆盖风险，不宣称已实现所有入口完整CAS。真实DB测试覆盖旧模型/执行位置/Host绑定重放被拒、native profile拒绝、失败revision不动、null/缺失同义及同目标参数保存。

## 验收

覆盖 local/remote/null 合法值与其他类型拒绝；metadata 拒绝执行位置；实际仓储模型+Host+展示字段同 revision 提交、冲突/失败保持原值；配置 GET/写回执一致。全部修改结束后统一测试，本文件落盘时未执行本批测试。

2026-10-15 统一验证已完成，89 项通过：`cargo test -p nola_application --lib conversation` 42 项，`cargo test -p nola_store --lib configuration_initialization_tests` 3 项，`cargo test -p nola_cli --lib conversation` 36 项（含真实仓储配置、跨端投影、大会话窗口），复用同 CLI test binary 的 `session_override_translation_tests` 5 项，`cargo test -p nola_cli --test conversation_configuration_web_e2e` 3 项。全部 `--test-threads=1`、`RUST_MIN_STACK=16777216`。无失败；仅已有 Apple ld `__eh_frame >16MB` 测试链接警告。`git diff --check` 通过，不等同移动端或 Mac 真机 UI 验收。
