> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 跨平台自动化：定义、发生实例与执行

核对：2026-11-08。自动化定义已不再只由不透明 metadata JSON 决定。当前类型与仍有差异的 Host 调度分开描述，不将旧“共享 typed executor 尚未开始”作为全部现状。

## 当前对象

| 对象 | 当前职责与所有者 |
| --- | --- |
| AutomationDefinition | Application 版本化定义；含 execution_spec、schedule_format、enabled、revision、metadata_json 与时间/tombstone |
| AutomationDefinitionSpec | Domain tagged union：Agent { spec } / UserTask { task }；不是旧计划中的 agent/source 两种假定 wire |
| AutomationExecutionSpec | Agent 的 prompt、执行位置、权限 profile、模型选择策略、context refs、Skill refs，显式校验 |
| AutomationUserTaskSpec | 提醒/生日/债务/约会/自定义事项；描述、目标时间、重复、十进制金额、完成及通知引用等 typed 字段 |
| Occurrence | 某个定义与计划时间对应的发生身份，独立于编辑定义和物理 Run |
| Execution lease/binding | occurrence、definition revision、请求摘要和真正 Run/执行身份的绑定；不能由进程句柄或 metadata 伪造 |
| Worker/OS wake | 唤醒、领取、续租、恢复观察与平台通知，不裁决 Core 执行终态 |

`metadata_json` 和 `schedule_format` 仍在当前模型中，供页面/来源适配与调度解释使用；它们不取代 `execution_spec`。CLI schedule adapter 读取已提交 typed spec 并核对回执，不因 payload 有 prompt 或脚本字段就发起另一类任务。Source/script 页面能力仍需按具体 adapter/Host 分别验证，不能假造不存在的统一 Source variant。

## 准入与结果

创建/编辑定义、发生时间计算、领取执行、真正准入 Run、观察/核对结果分别有边界。执行使用冻结的定义 revision/spec 和原 occurrence；已准入的 occurrence 只能恢复原 Run，不能以重试 HTTP 或启动失败为由再准入第二次。

launch_failed 与 admitted 后运行失败不同；准入结果未知先按原身份核对。worker exit、HTTP 200、工具结束和通知送达都不等于完成。执行记录/receipt 与正式 terminal seal 必须精确匹配；取消或租约过期不证明外部副作用已消失。

配置、metadata 或 UI 启用开关不授予工作目录、数据外发或未来编辑内容的权限。Agent/UserTask 不互相转换兜底，未支持类型明确保留不可执行状态。

## 发生时间与平台职责

`QueryAutomationOccurrences` 使用明确 IANA timezone、计划字段和有界区间计算 occurrence。系统调度查询未来区间，错过补跑按明确区间处理；DST 重叠/空洞规则以该共享查询为准，不在各 UI 重算。查询不重写已存 timezone、通知策略或 occurrence key。

平台只实现其真实可用的注册、唤醒、后台执行及恢复；iOS/Android 的 OS 限制与桌面 worker 不同。共享定义和 occurrence 查询存在，不能推导全部客户端都有持续调度或真机后台闭环。

## 源码与验证

- [Domain spec 与执行绑定](../../../../core/nola-domain/src/automation_execution.rs)
- [Application 定义/发生命令](../../../../core/nola-application/src/automation.rs)
- [执行准入](../../../../core/nola-application/src/automation_execution.rs)
- [发生时间查询](../../../../core/nola-application/src/automation_occurrence.rs)
- [Store 执行记录](../../../../core/nola-store/src/automation_execution_store.rs)
- [CLI schedule adapter](../../../../apps/cli/src/store/schedule.rs)

验收区分字段往返/CAS、时区/DST、重复 occurrence、领取/续租、崩溃前后准入、terminal 核对、通知和各端实际交互。不得使用用户计划或个人凭据作 fixture。本文只做源码核对，不添加真实模型或后台真机通过声明。

原设计、当时差距与测试观察保存在历史自动化记录（本地工作记录，不随仓库分发）。
