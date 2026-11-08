> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Nola 软件架构总纲

状态：重新发布 1.0.0 的现行职责；2026-11-08 扩展核对整个模块/部署入口，静态事实与目标/验收分开说明。实际依赖见[生产图](DEPENDENCIES.md)，接口见[当前接口目录](../模块/01-领域与协议/INTERFACE_CATALOG.md)，历史验证见[阶段归档](../../历史任务/2026-10-28_A2UI与跨端执行阶段归档/README.md)。
模块可使用值对象、trait、actor 或普通对象；选择依据是职责、可变状态和生命周期。

## 当前架构入口

先读[源码核对与差距](ARCHITECTURE_SOURCE_AUDIT_20261108.md)、[15 模块实现映射](CURRENT_IMPLEMENTATION_MAP.md)及[实际声明依赖](DEPENDENCIES.md)。状态机、facade、最终原生产物、Service/embedded、Store 与 Host 不因统称“Core”而拥有相同职责；逻辑目标 DTO 不能代替当前代码类型。

## 客户端与模型配置增量（2026-11-08）

原生 effort 选择、模板配置入口、检查器展开和移动转场的当前接线见[客户端交互与配置](../模块/13-客户端投影/CLIENT_INTERACTION_AND_CONFIGURATION.md)。展开/导航/动画是展示状态；模型默认、会话覆盖和 Run 冻结参数各有所有者。聊天不再用固定三档兼管提示词、工具与推理强度。底层 agent tier 及源 Host 合同保留其真实适用范围，不以更新文档制造协议兼容或数据迁移。

## 规范的归属

本总纲定义系统边界与术语；[所有权](架构决策与所有权.md)定义数据写权；
[公共协议](../模块契约.md)定义请求、提交与错误；[流程](跨模块流程与恢复.md)定义协作顺序。
模块专题细化自己的输入输出，不能另立写入者或重定义全局术语。
实施记录和版本数字以对应代码/证据为准，不把带日期的历史记录当作另一套现行规范。
出现冲突时须修正两处并登记任务，不能只靠“较新文档优先”长期容忍矛盾。

## 逻辑组件与依赖

```text
客户端展示 / CLI 布局
        │ 命令、查询、投影订阅
        ▼
Application 用例 ──► 纯 Core / Domain 决策
        │
        ├──► Store Port ──► canonical Store（事务、inbox、outbox）
        │                               │
        │                   已提交 effect / 领取与恢复查询
        │                               ▼
        └◄── 结构化回执 ── Runtime Supervisor / Driver
                                      │
                           Model / Tool / Platform Host
```

Domain 持有值、不变量与身份；Core 计算状态转换；Application 验证来源并协调事务；
Store 实施持久化完整性；Supervisor 管理资源与执行句柄；Host 执行 I/O。
Bootstrap 只装配这些对象。Presentation 拥有窗口状态和订阅，设备 DraftRepository 独立拥有尚未发送的编辑。
客户端查询经 Application，不因直接依赖某个库就取得其中所有业务写权。

## 身份与生命周期

| 名称 | 含义与边界 |
|---|---|
| Store/authority scope | 身份的命名域及权威存储，不由 UI 当前 URL 猜测 |
| Agent | 可复用配置及版本；没有聊天历史、运行任务或跨会话上下文，管理操作不创建对话 |
| Conversation | 单人/多成员统一的长期对话容器，拥有公共消息、成员关系、共享政策和任务关联 |
| Participant | 会话内成员身份及配置快照，引用 Agent 来源但不动态借用其最新配置 |
| Context | ConversationRef + participant 下的模型上下文；协调 Context 独立，摘要不覆盖原文 |
| Run | 一次已准入执行的持久身份及结果；封存后不能重新运行，继续任务创建关联的新 Run |
| Turn | Core 中一次交互/模型边界的语义身份；不得假设总与 Run 一一对应 |
| Effect | 已计划的逻辑外部操作；重新投递不等于获得重复副作用许可 |
| Delivery | 某次 effect 派发及执行所有权，受 generation/lease fencing 约束 |
| Attempt | 某次实际模型/工具尝试，失败、fallback 和续写分别计量 |
| Operation/Task/Goal/Plan | 各自具有语义与版本的工作对象，不能互用终态 |
| Window/Draft | 窗口显示与设备编辑，均不拥有已提交执行结果 |

配置编辑、草稿编辑可与运行并存，已准入 Run 使用冻结配置。
目标按 Participant/Context 明确主交互通道与 admission/排队/steer 规则；多成员分派不混用私有上下文，不同会话独立。当前源码仍有 agent_room 用例与独立映射，目标统一模型不能作为该实现已删除的证明。
子任务和维护操作有自己的身份与资源约束，不能靠全局 isRunning 表达调度策略。
WaitingResource 是调度准入/派发投影，不自动新增 Core WaitReason 或用户问题；资源获得后仍按原执行身份推进。
具体 Run/Turn 绑定必须读持久记录，禁止通过 ID 拼接或命名约定推导。

## 持久化与权威

同一 authority scope 内，canonical 业务事实由 Application 事务唯一协调写入。
桌面多进程通过 Application Service 共享写入权；移动端可嵌入同一用例边界。
“唯一 writer”指协调与事务权威，不强制一个全局锁、一种进程部署或一个 SQLite 文件。

消息正文的流式分块可以追加；用户编辑、撤回、分支和删除通过显式命令形成新修订/墓碑。
“保留原文”限制压缩不能破坏其来源，不阻止产品明确的删除和保留策略。
checkpoint 是执行恢复材料；完整历史、权限决定、usage 和任务事实有各自持久归属。
摘要、索引、Trace、远端镜像、UI 文本都不能替代源 authority。

跨设备执行中，执行侧 authority 拥有该 Run；接收侧 Store 保存验证后的 replica 与本地展示事实。
权威由明确 binding、source epoch 与协议确定，不能按设备时钟或最后更新时间选赢家。
跨 Store 不假设原子事务，采用持久请求、来源回执、去重与核对；本地投影失败不撤销远端已发生的副作用。

## 提交与外部效果

用户命令先持久准入，再派发 effect。Host 回执接受时在同一事务提交领域变化与后继意图。
查询不生成业务提交；通知提供已提交游标或带提交见证的有界 patch。
Accepted 只表示持久接收，Committed 才表示用例提交；两者都不等于模型/工具执行或 Goal 已完成。
进程在 COMMIT 后丢失响应时，客户端以原 request ID 查回执，不能当作未执行重发新命令。

数据库幂等保证同一业务接受不会重复应用；不能保证任意外部系统恰好执行一次。
外部操作结果未知时先核对，有显式风险重试授权才可按恢复协议重试。
业务取消、Host 停止确认、外部效果核对为不同事实。

## LLM、上下文与权限

主对话、压缩、辅助决策和子代理共用冻结配置解析、Attempt 身份、预算、取消与计量，
各场景保留自己的输出 schema 和输入投影，不共用万能可变管理器。
工作上下文由摘要块、未压缩尾部和必要权威引用构成；摘要候选与正式摘要分开，源核验后提交。
热分叉必须验证实际物理前缀与预算资格，缓存命中以 provider 计量证明。

工具能力声明、用户允许、执行所有权和 OS 授权独立。
权限按具体资源、参数、主体与作用域求值；scope 明确，撤销在派发与执行前核验。
内置确认处理应用授权；系统窗口只用于实际平台授权要求。Skill/MCP 文本不增加权限。

## 流式展示与计量

正式正文、思考/过程、工具与结果来自稳定 Part 和已提交投影。
模型/传输临时进度如需显示，使用独立、可丢弃的 live overlay，绑定原 Run/Attempt；
overlay 不写正文、不解除等待、不决定终态，并在对应提交或失败时核对清理。
封存不能重建整条消息或跳动滚动锚点，具体样式见 UI 设计目录。

每个物理 Attempt 只有一个计量 owner。父任务引用子任务费用，不重复计入会话总和。
usage 可迟到，修订原 Attempt；unknown、无缓存支持、真实零命中分别表达。
性能与质量要求按[验收标准](问题映射与验收标准.md)取真实基线；静态结构不代替真实体验验收。

## 模块设计与整改

15 个模块的输入输出和协议见[模块目录](../模块契约.md)。
按“封闭写口 → 源版本与取消提交 → 执行与调用 → 客户端封装 → 依赖收口”推进，
已实现事务、回执和恢复类型继续复用。未发布旧接口在调用者迁移后删除。
原总纲中的联调与版本日志已归档到[历史快照](../../历史任务/设计审查前快照/SOFTWARE_DESIGN_20261015.md)。

## 当前产品装配（2026-10-28）

`nola_agent_engine`承载现有agent/context/tool运行逻辑；`nola_core`是依赖engine的facade，不应按旧图认定所有逻辑都直接位于core包。Application直接依赖engine，并经Store Port协调持久化；bootstrap装配具体Store，Service为桌面多进程提供事务入口。实际声明依赖以[Cargo生产图](DEPENDENCIES.md)为准。

移动端由`nola_mobile_bindings`经现有C ABI/JNI装配embedded Application；同canonical数据库的handle共享runtime与准入锁。Web使用Rust Host HTTP/journal，Apple共享协议及渲染代码。模型位于远端API不表示执行authority也远端；paired复制的事实不能启动本地child。A2UI协议解释、动作决策和Surface业务状态归Core，各端只持有渲染与编辑瞬时状态。

源码可核对的模块入口与接口分层见[当前源码映射](CURRENT_IMPLEMENTATION_MAP.md)。这些说明不将目标交互、路由存在或依赖无环等同于全产品验收通过。
