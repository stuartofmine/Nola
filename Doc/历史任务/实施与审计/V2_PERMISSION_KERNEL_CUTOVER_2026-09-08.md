> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Permission evaluator Kernel cutover

本轮实际路径为 `AgentRuntime::tool_driver` 的权限评估 → Core `PermissionRuleSet` 薄适配 → `nola_kernel::evaluate_permission`。Core 的规则选择、origin 过滤、路径/bash/wildcard 匹配算法已删除，实际工具调度继续调用相同公开 Core API，后者调用纯 Kernel。

## 唯一类型与规则归属

Domain 接管六个原 Core 类型，字段与 serde 属性保持：`PermissionPolicyDecision`、`PermissionEvaluationReason`、`PermissionEvaluation`、`PermissionSubject`、`PermissionRule`、`PermissionRuleSource`。Core reexport 保持现有 checkpoint/FFI/调用者名称。`PermissionRuleSet` 仍在 Core，避免 Domain 依赖 Kernel；它仍负责构造/追加规则与合并 checkpoint conversation scope。

Kernel 输入是有序规则、subject、两个显式 authority facts，以及 `profile_confirmation_required` 布尔事实。Core 从实际 `AgentInputOrigin` 映射 authority，不根据 prompt 文字或工具名称推断实时用户权限。Kernel 保留最后一个可用匹配规则优先、原始 rule_index、完整匹配 rule 和 explanation；自动化来源排除的 conversation/turn 规则仍返回 Ask 与原 excluded-rule 原因。

匹配算法整体移入 Kernel，保留平台 `std::path::PathBuf`/component 语义，不调用文件系统、不解析真实路径或符号链接；file URL、percent 单次解码、parent traversal 拒绝与目录边界不改。通配符仍按原 UTF-8 字节实现，不能将此次移动声称为新的 Unicode glob 语义。

Bash scope 的 cwd、token prefix、command glob 和 scope 构造共享同一组 Kernel 纯函数。Core classifier 需要的 `safe_shell_tokens` 直接导入 Kernel，不保留第二份解析器。Kernel 仅新增 serde_json 用于原 scope JSON 解析；不依赖旧 Core 或 Host Protocol。

## 明确保留的风险分类边界

`recommended_profile_requires_confirmation` 及其破坏性调用分类仍在旧 Core，由 Core 算出布尔事实传入 Kernel。本轮没有把现分类换成粗糙新列表、减少危险操作确认或宣称整套权限决策已完全脱离旧 Core。用户明确规则仍按既有优先级覆盖 product-default guard。

独立审计发现旧 shell tokenizer 对双引号内 substitution 的处理缺口，root 在独立小范围补拒绝与回归；这不是 DTO 搬迁自然修复，也不应被迁移保真测试固化为正确授权语义。该修复的最终实证由 root 合并记录。

## 验收

Kernel 四项新增测试：120 种 authority/source/decision/profile 组合，原索引和 excluded 解释，bash cwd/复合命令/能力精确匹配，以及路径 URL/traversal/相邻路径/单次解码。旧 Core 原权限回归用于验证公开 API 继续同语义工作。

独立代理的 `core/tests/permission_kernel_migration_contract.rs` 迁移后 4/4 通过：十种实际 origin × 五 source × 三 decision 的 150 种组合、原 rule_index 和来源排除、四类路径能力、bash scope。它调用真实 Core wrapper，覆盖实际 origin→authority 映射，不只直接调用 Kernel。

## Root shell scope 修复

纯迁移之外，修复 shared `safe_shell_tokens` 的两处真实授权边界：

- 双引号中的未转义 `$`/backtick 仍有 shell 展开语义，不能继承静态脚本前缀 scope；单引号或正确转义的字面内容保留。
- 双引号内反斜杠只转义 shell 规定的字符，普通字符前的反斜杠必须保留，避免 `"review\.py"` 被误认为已授权的 `review.py`。反斜杠换行按 shell continuation 处理。

新增 `permission_quoted_expansion_contract` 3/3，通过实际 Core public wrapper→Kernel 评估。
含 `$()`、backtick、变量展开、引号拼接、奇偶反斜杠、单双引号的脚本身份与无法解析时只保留
精确 invocation scope。测试不执行任何 shell 命令。与独立迁移 contract 合计 7/7，
日志 `/tmp/nola-permission-quoted-final.log`。

## 实际 Application 授权生命周期

新增三项与原 batch 三项合计 6/6：once 只允许当前调用，后续同 scope 再次 Ask；conversation
同 scope 经冷重启可继续，但新 conversation 不继承；最终 receipt 故障不提前发布 ExecuteTool，
成功后完整 checkpoint/outbox/receipt 冷重放不重复。Custom instruction 仅证明不创建 grant、
不发布 ExecuteTool、冷重放一致，不代表 §20.2 已完成。

**已确认的下一优先缺口**：`intervene_permission` 当前把原工具标记 Denied 并移除请求；
全部工具终态时续推模型，仍有 Pending sibling 时可能停在 AwaitingTools 且没有 outbox。
这不满足 §20.2 “Custom response 挂起并恢复原调用”。下一生产切面必须设计并实现原调用
持续挂起、指令持久化与恢复的完整状态转换，不能只修复停滞症状或用现行为断言冒充验收。
Durable user/host 授权的真实执行层冷恢复也尚未由这三项新增测试证明。

旧 recommended profile 的 `capability.split('_')` 名称分类仍存在；它需要由可信 catalog 的
显式风险契约替换，不能继续扩展词表，也不能只删除检查而扩大默认授权。本轮 Kernel 仅接收
该旧分类器计算的事实，未声称整个权限系统已纯化或符合完整 routing guardrails。

最终 parser 版本 Application 全量 **263/263，0 失败、0 忽略**，
`/tmp/nola-permission-kernel-application-final.log`；workspace all-targets 编译通过，
`/tmp/nola-permission-kernel-workspace-final.log`。旧 Core 权限 24/24 再次通过，
`/tmp/nola-permission-core-final.log`。全部 root/子任务编译与测试句柄已终态。
完整 V2 目标保持未完成。
