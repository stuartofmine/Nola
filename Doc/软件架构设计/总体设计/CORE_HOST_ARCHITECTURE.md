> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Core、Application 与 Host 边界

状态：现行所有权约束；2026-11-08 核对实现分层及物理部署。总定义见[软件架构总纲](SOFTWARE_DESIGN.md)。

## 输入输出与唯一写权

Core 输入已提交状态、类型化用户/Host 事实和受控决策事实，输出 Transition：新状态、事件和 effect 意图。
这里的 Core 决策指 agent engine/kernel 的确定性状态转换；输入包含显式时间和身份事实，不让 reducer 自行选择物理资源。`nola_core` facade 与最终原生绑定不是同一个 crate，更不能把整个 Host/绑定层都称为无 I/O 的 Core。
Application 拥有公开用例入口、来源核验、命令幂等与事务协调；Host 不持有第二个公开可变 Core。
历史 AgentSubmission/AgentRuntime 名称表达内部 reducer 接口，不能当作绕过 Application 的客户端 ABI。

Domain/Kernel 定义领域值和纯决策，Application 通过 Store Port 提交。
Store 将 inbox、领域事实、checkpoint、outbox、计量接受与投影游标按同一用例事务提交。
每条命令只提交其相关对象，不要求修改任意配置也重写整份 checkpoint。
Driver/Supervisor 领取已提交工作、处理资源和取消，Host 执行实际请求，回执再由 Application 接受。

## 实现与制品所有权

| 边界 | 当前实现 | 不应沿用的描述 |
| --- | --- | --- |
| 状态机 | `nola_agent_engine` + `nola_kernel` | `core/src/agent` 是另一份独立 reducer |
| facade | `nola_core` rlib，重导出 engine 类型及纯辅助函数 | 由 nola_core 直接产出全部移动原生库 |
| 原生绑定 | `nola_mobile_bindings` staticlib/cdylib/rlib | 每个 Swift/Kotlin wrapper 自建运行循环 |
| 持久装配 | `nola_application_bootstrap` 选择具体 NolaStore，Service/embedded owner 持锁 | 薄客户端自己选择第二个业务数据库 |
| Driver | 通用 committed Host 循环，加 Goal/任务适配 | 整个 crate 已没有 engine 依赖 |

依赖事实以[生成图](DEPENDENCIES.md)为准，已发现差距见[源码核对](ARCHITECTURE_SOURCE_AUDIT_20261108.md)。

## 三类事件

| 数据 | 权威与用途 | 客户端行为 |
|---|---|---|
| canonical event / Part patch | Application 已提交的领域事实 | 按 owner、cursor、generation 幂等接受 |
| live progress / provider transport | 尚待接受的临时观察 | 可显示独立 overlay，不能制造正式消息或终态 |
| diagnostic Trace | 可丢弃诊断与追踪 | 不作为补原文、恢复任务或批准权限的来源 |

流式响应先绑定 effect/Attempt，再经 Core 与 Application 形成正式 Part 更新。
可按有界批次提交或发送提交见证 patch；View 不直接拼 provider 原文写回 Store。
订阅重连先补已提交游标，缺口不能以 Trace 猜测填充。

## Checkpoint 与恢复

checkpoint 保存当前恢复所需状态、来源引用、待办 effect、已接受水位及冻结配置。
正文可引用 canonical 内容块；恢复不能依赖 UI 缓冲或旧日志。
schema、Store、Service 与 ABI 版本各有代码中的唯一声明；本设计不维护一份容易失真的“当前数字”。
当前版本不兼容时显式拒绝，不能填默认权限或静默切换配置。

checkpoint 与相关业务事实由 Application 事务提交，不由 Host 的独立文件 effect 代替。
导出文件是已提交事实的派生材料，拥有 source revision/digest 和可重试导出意图。
恢复先发现持久未封存工作，再按原 Run binding 重建可用执行器；进程内 registry 是缓存。
新 worker 通过 fencing 接管，旧 worker 迟到结果按来源核对；租约过期不证明外部副作用结束。

## 配置与目录

会话配置可在 Run 进行中编辑，影响后续准入；活动 Run 保留冻结 invocation。
新目录版本不替换已派发调用的 schema。模型可发现新工具，但须经显式目录激活与请求边界生成新快照。
紧急撤销或平台不可用仍须在后续派发/访问时阻断，不能以快照冻结为由继续越权。
同一会话 Run 的绑定不根据窗口选中、模型显示名或当前全局默认重新解析。

## 权限、取消与完成

Core 决定应用策略，Application 保存等待/选择与来源，Host 返回 OS 和资源事实。
OS 拒绝不伪装用户 Deny，关闭窗口不伪装批准，恢复不重复申请同一已解决的决定。
本地取消信号可立即中止传输，但持久 Interrupt 必须先于其取消终态接受。
可确认停止与结果未知分别记录；不能要求所有外部效果可证实消失才允许封存带限制的结果。
无更多工具调用、finish_reason=stop 或 HTTP 200 均不单独证明任务完成；按 Core 结果/证据契约封存。

## 本地与远端

本地多进程和移动嵌入使用同一 Application 用例语义，不强制同一个二进制结构。
远端执行方的 authority 由准入 binding 固定；本地副本仅接受已认证来源的投影和计量。
同步不转移执行权或平台权限。authority 迁移须显式核对未完成工作并 fencing 原 owner。
不得按更新时间在本地 checkpoint 和远端镜像之间切换执行来源。

验收覆盖纯 Core、事务故障点、Host 实际效果、远端重连、客户端真实操作。
旧版二层分层与 Trace/原始 Core ABI 叙述保存在[历史快照](../../历史任务/设计审查前快照/CORE_HOST_ARCHITECTURE_20261015.md)。
