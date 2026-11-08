> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 记忆策略、上下文与远端执行边界

起始2026-09-28，状态更新2026-10-01。类型化上下文准入、远端意图/Host上限、
恢复及逐effect派发准入已接线并有分批验收；新增真实driver/Store及权限等待撤权
回归已证明派发前拒绝与Run收束。执行中即时撤权、全端持久执行政策和真实模型
总验收仍未闭环。累计证据及当前源码门槛见整体改造第8节，不能沿用旧结果验证新批次。
本合同属于 [整体模块化改造](../../../历史任务/实施与审计/MODULAR_REDESIGN_2026-09-28.md)。

## 最初确认的结构缺口与本轮整改范围

下列描述是改造前的基线，不是全部仍未处理；现有接线与限制见后续各节。

- mac 会话 memory override 会冻结到本地请求，但没有进入远端 Host 请求。
- CLI/Core Host 当前读取进程 `NOLA_LONG_TERM_MEMORY_ENABLED`，不是逐请求约束。
- Core 的记忆工具目录、resident recall 和自动提取会读取 MemoryPolicy，
  但直接注入的 Memory context 原先没有经过同一政策门禁。
- Web 的 long_term_memory 与 conversation.session_memory 原先都标记为 Memory。
  sessionMemory 是当前会话的摘要文本，不是开启记忆能力的布尔权限。
- iOS 把部分 durable preference/procedural memory 标为 UserPreference，会
  混淆已存储的记忆来源与用户当前明确提供的指令。

## 分离的类型与所有者

| 类型 | 含义 | 禁用长期记忆时 |
| --- | --- | --- |
| Memory | 跨会话存储/召回的记忆，包括记忆中的 preference | 不进入新 Turn 的上下文与模型提示词 |
| SessionMemory | 当前会话摘要或已压缩历史 | 保留，继续服从预算和摘要替代规则 |
| UserPreference | 用户明确提供的偏好指令 | 保留，不能仅因文字含“memory”被删除 |
| PersonalIntelligence | 平台提供的类型化个人上下文 | 不由长期记忆开关代替其独立授权 |

生产者按数据来源选择类型；Core 依据冻结政策准入，而不是匹配 ID、自然语言
关键词或内容。该门禁不是任意文本的保密分类器，不能阻止一个已获授权用户
主动把自己的文本作为普通消息或附件提供。

Core 在 start_turn 形成持久化上下文和模型请求之前调用同一纯准入函数。
被拒绝的 Memory 不能先混入 system message，再指望 adapter 从字符串识别。
平台展示预算可单独计算，但不能将展示层的估计当作执行授权。

## 远端请求与恢复合同

当前实现：Mac 将发送时冻结的 `memory_enabled` 明确传到远端，resume 仅携带
运行身份。CLI 严格解析可选布尔值，所有模型配置分支经过同一入口。
`RuntimeConfig.memory_enabled` 是客户端请求约束，不是 Host 权限上限；Host
独立配置解析由 `resolve_host_memory_policy` 负责。准入结果存入不可由 JSON
提供的 `admitted_memory_policy`，Core 新 Turn 消费该结果，执行策略摘要包含
完整策略。新提交清除上一 Run 的准入结果，但保留新请求意图。

恢复从 checkpoint 取原策略并检查当前 Host 的长期记忆及自动提取开关上限，
不以当前 UI 设置重新解释旧 Run。CLI 配置化 Host 在统一 dispatch 中再次
检查长期记忆/自动提取上限，位于 delivery 身份验证之后、ExecutionStarted
之前。拒绝作为不可重试的 ExecutionRejected 进入 canonical response，不能
仅抛出错误而遗留 pending effect；Cancel/CancelTask 不被该门禁阻断。
配置化非 fenced batch 同样走该入口，避免隔离批量执行绕过检查。

此检查只保证 effect 开始前的授权，不取消已经开始的流或工具，不等价于
执行中实时撤权；无运行配置的独立 driver fixture 也不证明生产授权完整性。

客户端只提交继承/启用/禁用的意图，不提交完整 MemoryPolicy、Host 数据源
授权或环境变量。Host 根据其独立配置得到权限上限，客户端只能收紧：
禁用必须关闭 long_term_enabled 和 automatic_extraction_enabled；启用不
能开启 Host 禁用项，也不能把 Host 默认关闭的自动提取打开。

默认开关与权限上限不能混为一物。若界面请求启用而 Host 不允许，应返回明确
的不可用结果或展示有效状态，不得让界面声称已启用但实际继续使用另一策略。
作用域必须绑定 conversation/run/submission，禁止修改共享进程环境变量来
模拟逐请求设置，避免并发会话互相影响。

最终 MemoryPolicy 应随 HostInvocation/权限快照原子提交；现有 Core checkpoint
中的策略用于恢复同一 Run，新的会话配置仅影响下一次明确提交。
恢复与每次 effect dispatch 还必须检查当前 Host 是否撤权：旧冻结值不能替代
有效授权。撤权后拒绝/结束待执行 effect，不能悄悄改参数或读取当前全局默认值
重新解释已冻结的运行。

## 验证门槛

1. Core 真运行：长期记忆禁用时，秘密 fixture 不在模型请求及active-turn
   context中；会话摘要、显式偏好保留；启用时Memory保留。
2. 多语言及含memory/search词语的ID/正文不改变类型决策。
3. 各平台会话摘要使用SessionMemory，durable records仍使用Memory；预算和
  摘要替代无退化。恢复旧turn中的已有文本并未由新turn准入自动解决。
4. 后续远端实现需覆盖Host禁用+客户端启用、Host自动提取关闭+客户端启用、
   客户端禁用、并发会话隔离、checkpoint重开、Host撤权、最终HTTP请求。

前三项的代码/定向证据不证明第四项完成，更不能证明记忆权限全链已闭环。

本轮结果：Core真运行5/5，预算/摘要替代1/1，CLI生产类型构造1/1，iOS模拟器
31/31，Android定向JVM5/5均通过；具体命令与报告记在整体文档。Android未做
设备/JNI验收。远端新增测试结果需单独记录，不能沿用这些通过数。

后续定向证据：CLI memory_ 34/34、真实 HTTP/Store 恢复+后续新 Run 1/1、
恢复套件8/8、dispatch边界12/12、MCP事务1/1通过。真实恢复测试曾暴露合法
配置提交后内存 config 未同步 checkpoint，现以 Application 提交结果更新
内存配置，未放宽 Core 校验。Mac 两次构建均因磁盘不足中断，测试尚未执行。
这组dispatch单测本身不等于撤权后完整canonical Run终态验收；后续真实driver/Store
及权限等待回归已补派发前收束，见下方Host准入模块证据，不能扩称执行中任意撤权。

### 已提交恢复状态的原子替换

恢复必须先准备候选 config 与合并后的 Host catalog，再由 Core 校验 checkpoint。
校验成功前不修改 live config、driver catalog、checkpoint 或 snapshot。提交来源
仍限于 Application 的已接受结果；这个缓存同步入口不是新请求的授权入口。
正常的 idle 配置更新应生效，错误 session 或无效 checkpoint 则四份状态保持
不变。生产入口与故障注入回归已接线，测试结果随整体文档记录。

补验结果：CLI恢复套件9/9通过；Mac四套定向26/26通过（0 failed/skipped），
真实加密HTTP请求解密验证明确false/true及原有取消/投影回归。此前测试错误地
读取信封外层，修复测试后重跑；未修改生产加密。报告路径与失败历史见整体文档。
这些结果仍不覆盖执行中即时撤权、全端权限统一或真实模型任务完成率。

### Host 准入模块的生产边界

记忆权限解析不应属于庞大的 CoreAgentHost 编排实现。抽取
`cli/src/host_memory_admission.rs`，负责独立 Host 配置、客户端约束、准入冻结、
恢复上限与派发前校验；不负责 Run 生命周期、模型调用、UI 或持久化事务。

实际消费者：native_bridge 在所有模型配置分支前做请求约束校验；runtime
在新 Run 准入、checkpoint 恢复时冻结/恢复；host invocation 将准入结果纳入
摘要；CliAgentHostDriver 在派发前检查当前 Host 上限。移除这些消费者对
core_agent_runtime 授权 helper 的反向依赖，不保留第二份策略实现。
Run 的重新提交重置仍由 runtime 控制，canonical 拒绝与终态仍由 Core/Application
处理。此抽取本身不证明 Run 收束正确，需另用持久化派发回归验证。

新增证据：在真实 SessionStore Begin 提交后撤销 Host 长期记忆权限，再进入实际
CommittedRuntimeDriver；验证模型端零连接、inbox保存ExecutionRejected且无
ExecutionStarted、Run终态、activeRun为空、pending effects/outbox清空。
此证据限定“模型派发前撤权”，不覆盖已经开始的流、任意工具副作用中途撤销。
Application另覆盖RejectedBeforeExecution与模型失败原因持久化、数据库重开
以及迟到ExecutionStarted不能复活旧delivery。

权限等待回归：真实loopback模型请求write_file，Host在实际permission resolver
等待边界撤权后返回Allow。后续ExecuteTool重新经过统一dispatch准入，目标文件
不落盘，Run终态且permissions/pending effects/activeRun均清空，没有第二次模型
请求。该测试证明标准Core权限effect与执行effect分离正确；不代表已运行的
delegated worker或device bridge内部批准通道均具备相同检查。
