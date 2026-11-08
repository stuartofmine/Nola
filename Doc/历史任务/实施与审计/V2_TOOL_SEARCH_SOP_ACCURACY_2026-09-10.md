> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Tool Search 与 SOP 准确性增量（2026-09-10）

本轮按 `ARCHITECTURE_V2.md` 的路由边界做一次横向初步优化：用户文本、语言和孤立词汇
不产生工具或权限。模型提交结构化 `tool_search`；Core 只在 Host 冻结的 capability catalog
中按声明的 id、title、JSON Schema 元数据排序；Host 执行时再次验证本轮可用集合。

修复两个实际契约缺口：

- 显式 `select:<name>,<name>` 现在能按请求顺序合并动态 Host/MCP schema 与静态内置
  schema，并一次激活完整集合。显式 select 不受语义搜索的 `max_results` 截断。旧实现把多个
  动态名称当成一个名称，或在匹配到首个动态 schema 后提前返回，导致同批静态 schema 丢失。
- 动态 schema 当前没有静态 `ToolSpec` 的 `category`、`read_only` 等筛选证明。带筛选的查询
  因此保守地不返回动态项，避免绕过调用者声明的筛选。后续若要支持，应扩展 Host 注入的类型化
  catalog descriptor，而不是从工具名称或描述猜测副作用。
- 模型调用未出现在该次采样 schema 中的名称时，Core 仍跳过执行，但现在把
  `tool_not_exposed_in_model_step` 绑定为必须完成的 catalog 恢复，并附带冻结目录 revision 与
  `tool_search` 恢复能力。下一模型步只开放 `tool_search`/`Skill`，成功发现后才能解除恢复门禁；
  旧实现虽返回错误结果，却未设置恢复门禁，模型可以直接结束 Turn。该判断只读取 Core 生成的
  机器错误码，不读取用户的语言、`search`、`current` 或任何自然跟进词汇。

持久 SOP 的模型恢复上下文升级为 `nola.active_sop_runs.v2`。除 pinned definition、run revision、
ready/active step 外，现在还声明工具执行血缘参数 `_nolaExecution` 的精确结构。模型必须从同一条
可信上下文复制 `sopRunId` 和 `sopStepId`；Core 在业务参数校验前提取该结构，Host 再验证 workspace、
run、固定版本和可执行 step。该结构只提供关联，不提供授权，也不能从工具名、普通文本或历史结果推导。

本轮 focused 回归覆盖动态+静态多选、动态 schema 不绕过筛选、SOP 恢复上下文暴露精确血缘协议，
以及未知/未暴露工具必须经 `tool_search` 恢复且不会重新暴露模型猜测的名称。多语言自然跟进和含
`search/current` 的普通错误文本也验证为不会触发 catalog 恢复。既有 Core catalog refresh 测试
继续覆盖未知工具等待 Host catalog 更新、checkpoint 后恢复同一 refresh effect，以及新目录到达后
恢复 Skill run。

仍需继续：动态 capability 的类型化筛选元数据应从 Host catalog 贯穿到 `tool_search` 输出；SOP
还需要真实模型 fixture 覆盖自然跟进、多语言描述、暂停/权限等待后的冷恢复，以及 verifier 失败后
不允许完成。任何补充都应扩展 catalog、协议或恢复状态机，不能增加语言关键词路由表。
