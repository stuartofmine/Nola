> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola 整体模块化改造与验证方案

P5正文日志与分块传输的目标合同见
[COMMITTED_ASSISTANT_JOURNAL_DESIGN.md](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_DESIGN.md)。
该专项的Domain合同、Store准备/发布/读取原语及Application planner/publisher已落地；
完整生产提交点、公开传输和客户端journal消费者尚未接通，不能把累计snapshot路径
说成已经线性化。累计交付与验收状态见第8节，逐批证据见第9节。
Canonical私有正文已接线的范围与实际规模证据见
[STREAMING_SCALE_ACCEPTANCE.md](STREAMING_SCALE_ACCEPTANCE.md)，不等于公开journal迁移完成。
Core控制状态与私有恢复正文的下一实施合同见
[CORE_CHECKPOINT_CONTENT_CONTRACT.md](../../软件架构设计/模块/04-存储与索引/CORE_CHECKPOINT_CONTENT_CONTRACT.md)；版本化恢复尚未接线。

Goal起始：2026-09-28。最近更新：2026-10-04。状态：多项模块改造已落地并分批验收，
整体迁移仍在进行中；最新公共语义分层回归已通过，另有CLI终态汇总回归暴露的问题
已修复，短stream deadline与CLI完整binary/指定PTY也已重验。未将局部通过扩称为
全平台/整体验收完成；完整证据与剩余门槛见第8节。10-02完整publication-set、
有界多帧planner及canonical owner借用映射的六crate lib、完整Application/Store/Domain
tests及显式规模重验已通过；CLI完整重跑1,408通过/2忽略，未启用journal消费者。
10-04续批已接入真实提交的显式Turn/Part归属与窄规划端口；初始内容完整性、独立
Core身份和internal handoff历史读取正在统一整改，最新证据与边界见8.12。

生成参数覆盖、seed 三态及条件采样边界见
[GENERATION_OVERRIDE_CONTRACT.md](../../软件架构设计/模块/08-模型配置与调用/GENERATION_OVERRIDE_CONTRACT.md)。

记忆政策、会话摘要分类及远端执行缺口见
[MEMORY_EXECUTION_POLICY_CONTRACT.md](../../软件架构设计/模块/12-记忆数据源与自动化/MEMORY_EXECUTION_POLICY_CONTRACT.md)。

## 1. 目标与文档关系

目标是让同一用户意图经过一个确定的配置解析、授权、执行、提交和呈现链路。
模块化以状态所有权、接口和独立验证为单位；文件变短、增加 `parts`、增加 Facade
本身不算完成。项目尚未上线，不为废弃执行链路保留双写、兼容推断或旧状态机。

本方案落实 `ARCHITECTURE_V2.md`、`ARCHITECTURE_V2_MODULE_CONTRACTS.md` 和
`../../软件架构设计/模块/03-Application与事务/CANONICAL_CONVERSATION_TRANSACTIONS.md` 的单一事实源原则；验收证据沿用
`../../软件架构设计/模块/15-观测与验收/TESTING_ARCHITECTURE.md`。它是当前实施顺序和缺口清单，不宣称旧计划中的条目已完成。
冲突必须同步修正规范，例如旧模块合同要求 Mac 目录访问一律弹系统选目录窗口，
与当前用户要求的内置确认交互不一致：产品批准与 OS 实际授权应独立，只有 OS 确实
需要额外授权时才由平台适配器获取，不把文件选择器当作普通批准 UI。

## 2. 当前证据与根因

以下为本批改造前的工作树静态基线，不能替代运行验收。已迁移项见第 8 节。

| 证据位置 | 观察 | 结构问题与后果 |
| --- | --- | --- |
| `NolaMac/Features/Chat/MacWorkspaceModel.swift` | 约 4,926 行，持有流 assembler、Part reducer、Trace reducer、会话、配置与运行控制 | Presenter 同时持有执行和投影状态；需逐条确认使用者并删除重复内容路径，不能仅移动到 extension |
| `Nola/Services/ChatManager.swift`、Android `ChatViewModel.kt` | 分别约 3,159 / 2,346 行 | 多平台各自承担配置、运行、持久化和显示，跨端语义容易漂移 |
| `cli/src/llm.rs` | 三个 `include!` 拼接到同一作用域 | 已拆文件，但 HTTP、协议、重试和流状态未获得独立所有权 |
| `core/nola-application/src/model.rs`、`core/src/agent/reasoning.rs`、Mac `MacReasoningConfiguration.swift` | 同一 effort、budget、输出限制有多处校验 | UI 可保存的配置和运行时可执行配置可能不一致 |
| `core/nola-application/src/conversation_configuration.rs` | 接受原始 JSON，仍接受 Mac 已删除的 reasoning/default 字段，缺省协议可从 control 推断 | 会话设置、模型能力、协议选择与 UI 偏好混合，缺少一次解析后的运行配置 |
| `core/nola-application/Cargo.toml` | 经过 `nola_legacy_agent_adapter` 依赖旧 Core | 迁移接缝已命名，但旧引擎仍是事实依赖，不能称纯 Kernel 迁移完成 |
| `core/nola-domain/tests/workspace_dependency_architecture.rs` | 只扫描 `[dependencies]` 文本段和依赖键名 | target/build/renamed 依赖可漏检；绿灯不等于真正遵守层次 |
| 既有测试与上轮执行 | 快速验证、真实模型、UI 证据已有设施；全量 Mac 执行曾被人为中止 | 必须分别记录通过、失败、运行中、超时；启动和部分通过不构成完整验收 |

根因是同一决策被多个所有者解释，以及接口容许调用者绕过所有者。
大文件是风险信号，不能仅凭行数认定每段代码都错误。

## 3. 参考实现与 OpenAI Docs 的设计依据

已阅读本地资料的实际接口，而非仅按项目名称类比：

| 资料 | 采用的原则 | 不直接照搬的部分 |
| --- | --- | --- |
| `参考资料/codex-main/codex-rs/codex-api/README.md` | HTTP/认证注入/重试/SSE 属于协议适配器；上层消费 typed event | 不要求 Nola 仅支持 OpenAI |
| `参考资料/codex-main/codex-rs/thread-store/README.md` | canonical history append 与 metadata patch 分离；活动线程有明确持久化所有者 | 不引入其 JSONL/SQLite 兼容双轨 |
| `参考资料/codex-main/codex-rs/app-server/README.md` | thread/turn/item 标识、开始/结束生命周期、批准与恢复是显式协议 | Nola 继续以 Application 提交后的投影作为产品事实 |
| OpenCode `session/processor.ts`、`permission/schema.ts`、`permission/evaluate.ts` | processor 持有运行内状态；权限有独立身份与决策模块 | 不用通配字符串替代 Nola 的规范化资源授权 |
| `参考资料/claude-code-system-prompts-main/README.md` 及 sandbox/memory prompt | 将工具、环境、记忆等提示分成有来源的片段，区分单次权限和后续请求 | 此资料是第三方提取的提示词，不是 Claude 内部架构的权威证明；提示词也不能替代执行器校验 |

官方文档核对日期：2026-09-28。

- [Agents 产品边界](https://developers.openai.com/api/docs/guides/agents)：SDK 会话、
  Responses conversation、运行环境是不同资源。Nola 保持自有本地运行时和存储，
  不将 provider session ID 作为本地会话或权限身份。
- [Conversation state](https://developers.openai.com/api/docs/guides/conversation-state)：
  历史包含消息、工具调用和工具结果，不能把最终显示文本当作完整续接状态。
- [Function calling](https://developers.openai.com/api/docs/guides/function-calling)：
  调用与结果通过 call ID 关联；模型选择工具，执行器处理具体调用与结果。
- [Guardrails and human review](https://developers.openai.com/api/docs/guides/agents/guardrails-approvals)：
  校验和批准是不同控制，批准暂停/恢复必须绑定待执行调用。具体批准策略仍由
  Nola 的用户设置及资源事实决定，不把文档样例中的策略变成额外弹窗。
- [Agent evals](https://developers.openai.com/api/docs/guides/agent-evals)：
  trace 可用于诊断决策，再转化为可重复评估；trace 不因此成为 Nola 的 UI 数据库。

## 4. 模块所有权

```text
Mac / iOS / Android / CLI / Web Presenter
        -> Application Client (command / query / subscription)
        -> Application (admission / config resolution / transaction)
        -> Kernel (pure decisions) + Domain (typed contracts)
        -> Store Port <- Store (atomic commit + effect outbox)

Runtime Supervisor -> claimed effect -> Provider / Tool / Platform Host
                   <- correlated result -> Application result command

Committed projection -> Presenter -> Markdown / timeline / interaction views
Committed diagnostics -> Observability / evaluation (no product write-back)
```

| 模块 | 唯一拥有的状态/职责 | 输入输出 | 禁止行为 |
| --- | --- | --- | --- |
| Domain | identity、资源、生成参数、不可变投影和值对象不变量 | typed values / typed validation errors | IO、时钟读取、provider SDK、UI 和 SQLite |
| Kernel | Turn/Call/Permission/Compaction/Completion 的纯状态转换 | state + facts -> events + effects | 写数据库、读文件、根据语言关键词路由 |
| Application | 用例、配置优先级、revision、事务边界 | command -> receipt；query -> projection | 直接 HTTP/进程；公开任意数据库写接口 |
| Store Port / Store | 持久化、CAS、outbox、索引与恢复读取 | typed repository calls | 决定工具、权限策略或模型回答 |
| Runtime | effect claim、交付、取消、进程生命周期 | effect -> correlated result | 拼装正文、改写权限事实、旁路提交 |
| Provider adapters | endpoint/auth、wire request、SSE、provider continuation | FrozenModelRequest -> ModelEvent | 解释用户任务、写会话、产生 UI 标题 |
| Tool/Platform Host | schema 对应的具体工具执行及 OS 能力 | authorized invocation -> typed result | 自行扩权、从输出字符串猜权限、构造历史消息 |
| Presentation | scroll anchor、输入状态、渲染缓存、交互展示 | projection + local view state | provider delta 作为历史；Trace 修补消息 |
| Observability/Evals | 脱敏 trace、性能、证据归档 | committed observations | 成为完成状态或 UI 内容事实源 |

## 5. 核心数据与配置设计

### 5.1 一次解析的模型配置

区分四类数据：`ModelRoute`（端点/协议/模型身份）、`ModelCapabilities`
（支持什么）、`GenerationPreferences`（用户想要什么）、`ResolvedGeneration`
（这一轮具体发送什么）。密钥只以 credential handle 进入运行边界，不存进配置快照。

优先级固定为本轮明确覆盖 > 会话当前模型覆盖 > 模型默认 > 产品默认。
缺省继承、显式零值、清除覆盖必须可区分。解析发生在 admission；快照包含模型和
配置 revision，在整个 Turn 内不受用户切换 UI 或后台目录刷新影响。
不可支持的明确配置返回 typed error；未知能力不伪装为支持。

三档“简洁/中等/复杂”是用户偏好，effort 和 budget 是解析后的协议参数。
回答呈现与推理预算是不同字段，产品可以用同一个档位同时设置两者，但不得通过
professional/casual 等语气推断推理强度。档位映射必须在共享配置模块完成，所有端
消费同一结果。共享Domain映射与Application准入已实现，Mac主提交、会话参数和模型
测试页已接同一解析；iOS已接共享输出准入与冻结能力。全端三档UI、完整参数快照及
恢复重建尚未全部统一，不能将其中一端的接线扩称为全端完成。

### 5.2 会话与内容

Conversation、Turn、Message、Part、Call、Effect、Attempt 各自有稳定身份。
provider item ID 与本地 Part ID 显式关联。追加、替换、终结有独立操作与单调 revision。
最终快照和实时刷新都读取同一投影；重放和恢复不能重复创建 Part。
错误终态、取消、等待批准和模型完成不同；网络 EOF 不能当作模型完成。

### 5.3 权限

权限是 `actor + capability + normalized resource + operation + scope + policy revision`
上的决定，批准必须绑定具体 call/effect。OS 可访问性、用户批准、工具可见性、默认 cwd
分别管理。授权回调乱序、重复、取消后迟到必须幂等且不能复活请求。
用户提到 memory、Web、search、current 等词不产生路由或授权。
工具不在目录、参数无效或能力不可用应返回模型可恢复的结构化结果。

### 5.4 上下文、记忆与完成

ContextSnapshot 记录输入 items、token 估计来源、已提交压缩版本和选用的记忆来源。
压缩不丢失 call/result 对、未完成任务、权限等待、产物引用；续接只使用已提交版本。
长期记忆默认策略与上下文压缩独立，写入带来源、作用域、启用状态。
任务完成由真实运行证据与用户目标决定；独立审查是否调用由模型决定，不强制所有
任务执行审查。UI 展示当前状态，不推断任务是否成功。

## 6. 实施顺序与删除目标

每批先补能暴露缺陷的合同验证，接通真实生产消费者，再删除旧入口。
不在无关 dirty worktree 上回滚已有改动，不修改生产数据完成迁移测试。

| 批次 | 产出与删除目标 | 完成门槛 | 当前状态 |
| --- | --- | --- | --- |
| P0 证据与边界 | 本文、依赖图检查覆盖 target/build/alias；记录旧接缝 | mutation fixture 能证明禁止依赖会失败；真实 workspace 通过 | 已完成首批依赖门禁，并补LLM语法级单向依赖与无环检查；不是全仓库所有语义依赖已解环 |
| P1 配置领域 | 生成参数移入 Domain；Application 与运行时共用约束；完整冻结执行配置 | 核心参数边界、序列化、无效配置不提交；生产消费者编译通过 | 多项已落地并验收：共享参数/能力/三档准入、远端profile意图、seed三态、HostInvocation v7、Mac/MLX与iOS准备/凭据；完整全端快照及冷恢复未完成 |
| P2 Provider | 用独立模块替换 CLI LLM `include!` 共享作用域；统一协议 continuation 和错误类型 | Responses/Chat/Anthropic request + stream + tool continuation + retry 合同 | 多项已落地并验收：私有模块与依赖解环、纯执行token、输出上限、协议/URL解析、MLX请求与响应writer；真实模型与完整fallback能力合同仍待验收 |
| P3 权限与工具 | 一个 ToolInvocation/PermissionDecision；清理各端资源推断和重复批准路径 | deny/allow/取消/乱序/重放/重启；真实 Host 和 UI 都覆盖 | CLI派发/结果egress校验、iOS plan刷新/取消fence、iOS与Android delivery身份修正已有分批验收；全端稳定权限策略、执行中撤权及UI验收未完成 |
| P4 Run 与投影 | 分离 Mac run coordinator、configuration、interaction、projection reader；移除 UI 内容 reducer | 单次提交、连续追加、冷启动恢复、停止等待批准；截图/AX/时间线 | Run/订阅/呈现session/提交快照与门闩/恢复/取消投递/原文reader已拆并接真实消费者，多批Mac回归通过；app级后台Run、完整交互所有者与冷重启恢复仍未完成 |
| P5 其他端接入 | iOS/Android/CLI/Web 使用同一配置、权限和 projection 合同 | 同一语义 fixture 跨端结果一致；各端构建/测试 | 私有Text增量writer→reader→replica已接通并规模实测；公开journal的Domain/Store/planner已实现，未生产切换；最新全字段公共语义及CLI指定交互回归已分层验收，非全平台整体验收 |
| P6 上下文与完成 | 统一预算/压缩/记忆来源/完成证据；清理 legacy adapter | 压缩后工具续接、长任务恢复、模型自行审查选择、真实模型任务完成 | 不再是完全未开始：类型化记忆准入、Host上限/恢复/逐effect拒绝与Core部分复制消除已落地；增量私有恢复、legacy清理、全链复杂度与真实模型任务验收未完成 |

后续批次须在修改前列出实际调用者与将删除的接口。不能仅创建未被使用的新模块后
把迁移状态标成完成。P1 全面完成前不能宣称模型配置已经全端统一。

## 7. 验证矩阵和证据

| 层次 | 证明什么 | 主要入口 |
| --- | --- | --- |
| 依赖/纯领域 | 禁止依赖、typed 参数边界、非法值拒绝 | `cargo test -p nola_domain` |
| 应用事务 | 配置优先级、原子提交、CAS、幂等、失败不改历史 | `cargo test -p nola_application`，按修改合同选择 integration target |
| 协议适配 | 原始 SSE 分片、tool call/result、reasoning continuation、错误和取消 | `cargo test -p nola_cli` 的 provider 模块用例，必要时本地 HTTP fixture |
| 平台呈现 | 追加顺序、权限交互、Markdown、滚动锚点、恢复一致 | `scripts/test_mac_layers.sh` 相应层；iOS/Android/Web 原有入口 |
| 真实模型 | 自然追问、多语言、未知工具恢复、压缩后继续、目标完成 | `scripts/run_real_system_tests.py` 及 `real_e2e_matrix.json` 中匹配场景 |
| 全量回归 | 所有受影响平台与 workspace 的整体状态 | 在候选改造里按批次执行；不能用定向绿灯替代 |

每个结果记录命令、当前源码版本/差异、退出状态、报告路径、覆盖范围。
昂贵流程保留运行句柄并持续观察，不因一次观察超时重启，不因耗时而把未完成记作通过。
没有凭证/设备的真实模型或 UI 场景标记为缺失证据；本地 fixture 不改称真实模型验收。
基础回归可覆盖有限输入，产品路由不能反过来按这些输入的关键词硬编码。

## 8. 本轮goal累计交付总览（2026-10-02）

本节覆盖2026-09-28开启本轮goal以来的工作，不只统计最后一批。范围依据下方实施
记录、实际生产调用者和测试证据；不把goal开始前已有的脏工作树/UI改动计入，不用
整个`git diff`的文件数冒充本轮交付量。多批测试大量重叠，不累计相加成唯一测试总数。

“已落地”表示实现已接实际调用者；“已验收”只在标明的源码时点与测试范围成立。
“源码已接线、待验收”不等于功能已完成，也不能继承修改前的绿灯。旧节标题中的
“实施中/尚未验收”是当批历史，累计状态以本节及最新批次为准。

### 8.1 已落地的主要架构变更

| 模块 | 本轮累计变更 | 实现/合同入口与边界 |
| --- | --- | --- |
| 分层与依赖门禁 | 使用Cargo metadata真实package identity检查target/build/alias；Kernel隔离；CLI LLM增加Rust语法级允许依赖与无环门禁 | `core/nola-domain/tests/workspace_dependency_architecture.rs`、`llm_module_architecture.rs`。门禁不替代全仓库职责审查 |
| 共享模型配置 | generation、protocol、execution capabilities、输出限制、reasoning与三档映射移到Domain；Application负责最终合并/准入；输出上限不再被上下文预留clamp改写 | `core/nola-domain/src/generation.rs`、`model_capabilities.rs`、`reasoning_tiers.rs`；Application `model_generation.rs`。缺省/零值/清除、能力未知与明确不支持分开 |
| 会话覆盖与远端意图 | 保持本轮>会话>模型>产品优先级；远端按host-owned profile ID/revision提交，跨host同名档案隔离；seed继承/清除/指定值三态贯通Rust/Mac/Web | `run_generation_intent.rs`、`generation_override.rs`及[覆盖合同](../../软件架构设计/模块/08-模型配置与调用/GENERATION_OVERRIDE_CONTRACT.md)。过期/删除档案拒绝，不回落全局endpoint |
| Provider与执行控制 | 删除CLI LLM文本include，拆独立contracts/transport/stream_state/diagnostics/provider/编排；纯ExecutionContext脱离终端；协议分派、输出字段、continuation和URL构造消费一致事实 | `cli/src/llm_parts/`、`execution_context.rs`。三协议真实loopback HTTP验证参数，不等于联网真实模型验收 |
| 本地模型与MLX | 区分profile/artifact/live process实例；复用核对启动事实；Mac冻结endpoint并拒绝准备期间实例变更；MLX请求级参数、JSON/SSE响应codec与单一HTTP writer分离 | `cli/src/local_models.rs`、Mac endpoint/launch snapshot、`NolaMLXRuntime/MLXGenerationRequest.swift`、`MLXChatResponse.swift`、`MLXHTTPResponseWriter.swift`。尚未完成长期runtime lease及权重内容快照 |
| 精确执行与凭据 | HostInvocation推进到v7、OutputAdmission到v5：协议/最终输出字段/参数/重试/并行/上下文/HTTP或native route/secret来源有明确身份；增加精确Run只读查询；iOS版本化凭据先准备再CAS发布 | `core/nola-host-protocol/src/host_invocation.rs`、`host_secret.rs`、`Nola/Services/IOSCloudCredentialOwner.swift`。digest不是可取回凭据；iOS完整冷恢复仍关闭 |
| 权限与设备执行 | CLI在派发、接收和发布device结果时重验同一egress证据与原期限；nonce-scoped清理；iOS single-flight plan刷新、原Run/request回调fence与可撤销dispatch permit；修正两移动端model attempt≠physical delivery attempt | `cli/src/web_parts/native_bridge.rs`、`device_egress_tests.rs`、`IOSRunToolPlanSession.swift`、`IOSRunExecutionPermit.swift`、Android `HostDeliveryIdentity.kt`。刷新不是延长同意，未宣称全端即时撤权 |
| Mac状态所有者 | RunCoordinator拥有运行身份/终态；ObservationCoordinator拥有订阅lease；PresentationSession拥有accepted/visible正文与待展示队列；SubmissionSnapshot/Ticket冻结意图并在实际dispatch竞争 | `NolaMac/Features/Chat/MacRunCoordinator.swift`、`MacProjectionObservationCoordinator.swift`、`MacAssistantPresentationSession.swift`、`MacRunSubmissionSnapshot.swift`、`MacRunSubmissionTicket.swift`。准备时取消不偷偷创建Run，暂停/重挂不丢待显示正文 |
| 终态、恢复与取消 | 本地/远端都要求精确持久终态证据，EOF/Trace/取消HTTP成功不等于完成；原启动快照恢复与精确lease栅栏；取消意图/投递分开，专用cancel-only driver不发新模型/工具调用 | `run_recovery_inspection.rs`、`run_cancellation.rs`、`cancellation_delivery.rs`、`cli/src/cancel_only_driver.rs`及Mac恢复/取消服务。关闭窗口不等于取消Run |
| 上下文与记忆 | 按来源区分Memory/SessionMemory/UserPreference；关闭长期记忆不删除会话摘要或用户指令；Mac会话继承/启用/禁用放Inspector；Host独立上限、恢复及逐effect准入，拒绝进入canonical并收束Run | [记忆执行合同](../../软件架构设计/模块/12-记忆数据源与自动化/MEMORY_EXECUTION_POLICY_CONTRACT.md)、`cli/src/host_memory_admission.rs`、Core context admission。权限等待期间撤权后零文件副作用已有受控HTTP/Store回归 |
| Canonical正文与历史读取 | 统一CanonicalFieldReader；原文text/json双字段的分支、复制、归档、搜索、导出与历史语义读取接线；明确Absent/Complete/Deferred；Text生产Append改为head CAS+新增chunks，Close一次封存 | [Canonical流式合同](../../软件架构设计/模块/13-客户端投影/CANONICAL_STREAMING_TEXT_CONTRACT.md)、`canonical_field_reader.rs`、`canonical_stream_store.rs`、`canonical_stream_read.rs`、`canonical_part_projection.rs`。保留原JSON字节/数值，不以预览充全文 |
| 公开journal基础设施 | Domain严格typed operations/内容codec/独立时钟；Store同UoW提交证明、manifest/chunk、固定水位snapshot、release与byte range；完整有序多owner/多frame set、唯一writer及schema33集合凭证；Application有界分帧与canonical owner借用映射 | [journal设计](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_DESIGN.md)与[Application adapter](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)。Text/Reasoning/Tool/Source/Citation/Attachment/Step模块已实现；10-02六crate lib已验收，不是所有实际提交点已生产发布 |
| 原文披露、Markdown与reader | review未批准正文在普通历史/搜索/预览/原文入口一致隐藏；Part final与can_read_original分开；Mac原文查询拥有真实取消token和迟到响应fence；Markdown tail boundary scanner处理流式尾部边界 | [原文合同](../../软件架构设计/模块/04-存储与索引/ORIGINAL_TEXT_READER_CONTRACT.md)、`MacOriginalTextReadModel.swift`、`MacMarkdownTailBoundaryScanner.swift`。未把goal前所有Markdown/UI改动归入本轮 |
| Core热边界复制 | checkpoint投影可消费owned state；事件封装消费Vec而不是再次克隆累计Part；checkpoint小metadata直接读取，不为四字段构造全文snapshot | [私有checkpoint合同](../../软件架构设计/模块/04-存储与索引/CORE_CHECKPOINT_CONTENT_CONTRACT.md)。只消除这些具体边界，累计Part、每delta回滚及checkpoint正文序列化仍存在 |

### 8.2 公共语义整改：10-01批次实现与验收

最新批次不是单纯UI补丁，而是补齐Domain→Application→Store→客户端的公开数据合同：

- Text保留review控制事实并隐藏未批准正文；Reasoning title与detail独立。
- Tool title、人类summary、structured result和output preview同时保留，明确缺失、null、
  空字符串和空集合；origin/permission-display、artifacts及operation、时间戳不丢失。
  `ToolProgress.detail`按真实Core更新修改preview，不误写人类摘要。
- Source仅公开严格typed、脱敏的MemoryUsage证据，不含记忆正文/摘要/凭据；Citation
  保留真实ID、MIME、section/location与range；Attachment保留operation。
- `full_output`只有message/Part/call三身份只读描述，不是授权，不能把私有文件路径
  变链接或捏造读取/批准按钮；canonical唯一大structured result不能被诊断预览截断。
- 普通projection、journal planner、Store semantic source与Kernel校验已修改；Mac/iOS
  已迁移真实DTO、presenter和工具卡，补实际Application提交→查询→冷重开用例。
  Web两条实际展示链均已接；Android新typed模型/decoder/projector、实时消息和Compose
  工具/来源/附件卡也已接线并冻结，新增8项JVM回归；完整debug JVM测试已实际通过。
- iOS public StructuredValue复用Core的AgentCoreJSONValue精确整数codec，消除另一套
  Double-only decoder造成的工具input/output及历史字段舍入。原小值/有限小数显示
  不变；literal JSON→presenter/details及真实live commit→query→overlay→冷重开验证
  UInt64.max、Int64边界、±(2^53+1)与嵌套值，不经Double/NSNumber提前舍入fixture。
  旧revision/context等Int64字段未在本批整体迁移，不宣称全iOS时钟域都支持最大u64。
- 已修共享历史读取中的降级：Store明确验证的完整公开值，经Application还原严格
  Domain Part；缺失/null、独立富字段及本地revision保留。只接受精确canonical owner、
  ContentJson来源和完整字段；Deferred、未知/私有字段及不完整证据不伪装为完整工具。
  真实remote admission→commit→full/window/page→冷重开和大结果原文回归已通过。
- Part诊断已拆独立模块，先限制借用字符串/键/格式化访问，再执行整条事件的实际编码
  总预算；Created不重复存两份snapshot。此合同只属于diagnostic，canonical原文和
  回放不消费预览，非Part事件与Core累计状态的剩余成本没有因此消失。
- Web旧JSON传输对已知MemoryUsage超出safe integer范围的数字明确拒绝，不静默舍入；
  任意Tool structured JSON数值的全链无损仍是缺口。正式journal codec/transport必须
  保留原数字token，不能把229项前端通过写成全部u64/JSON数值问题已解决。

10-01富字段批次准确验收状态（10-02新源码重验见8.5）：

| 范围 | 10-01实际结果 | 不能据此宣称的内容 |
| --- | --- | --- |
| Web旧界面浏览器运行时 | `/tmp/nola-public-part-web-runtime-r2-20261001.log`：**48/48通过**，包含富字段、review、三身份与evidence边界 | 非联网真实模型或全站视觉验收 |
| Web前端完整测试 | `/tmp/nola-public-part-web-full-20261001.log`：**229/229通过**；`/tmp/nola-public-part-web-check-r2-20261001.log`：**0 errors / 0 warnings** | 229是本次suite总数，不是本轮新增测试数量，也不含原生端 |
| Rust公共语义联合回归 | `/tmp/nola-public-semantics-libs-r6-20261001.log`：**exit0，Application216/Core396/Domain90/Kernel121/Store277/StorePort3，共1,103通过、1默认忽略** | 完整六crate lib，不是全workspace；默认忽略的规模测试未在本批重跑 |
| 独立Core wire oracle | `/tmp/nola-public-semantics-wire-oracle-20261001.log`：**1/1通过**，对照实际共享/Core类型与bounded nested合法域 | lib不会运行此integration target，必须独立记证据 |
| Application完整tests | `/tmp/nola-public-semantics-application-integration-r3-20261001.log`：**exit0，77组、lib216通过、integration252通过，共468通过/0失败**；包含真实remote冷重开、大结果原文与严格来源回归 | 首跑6失败、二跑1失败均保留；修正真实fixture身份及旧一律Historical断言后完整重跑，不以lib绿灯替代integration |
| 已闭合的失败 | 原三个historical_context_references fixture和诊断13,334字节失败已在r6闭合；保留更严格的**编码字节<12,000**原断言，增加通用serializer与真实类型事件回归 | 这是12,000-byte上限，不是把阈值放宽到12KiB；不截断canonical结果 |
| Mac最新富字段 | `/tmp/nola-public-semantics-mac-r4-20261001.log`：实际重新build/test，**56 XCTest+14 Swift Testing=70通过/0失败**；xcresult独立确认70/70 | 首两次编译失败、三跑历史降级及旧fixture失败仍保留；七个过滤类不是全部Mac测试，也不是视觉/联网模型验收 |
| iOS最新富字段 | `/tmp/nola-public-semantics-ios-r5-20261001.log`：七类实际模拟器build/test，**64通过/0失败/exit0**，独立xcresult确认；含顶层JSON shape、统一精确整数、真实live/cold回归 | 首跑null失败及新fixture非法origin/越权terminal失败均保留；不放宽合同。remote生产仍仅terminal提交后同步canonical富字段，完整可信冷恢复仍关闭 |
| CLI完整binary | `/tmp/nola-public-semantics-cli-bin-r2-20261001.log`：**1,403通过/0失败/2忽略/exit0**，含新增11项consumer/deadline/Markdown边界、原terminal34/activity11和未批准review不进正文gate | 首跑1,394通过/9失败保留；修正真实owner/admission与canonical reader测试合同后完整重跑。两个Worker E2E忽略未执行，不是完整CLI integration suite |
| CLI指定交互PTY | 普通单帧stream、19字符后暂停、queued follow-up、dynamic resize/cancel、permission-return及fifty-read **六项各1/1通过**；fifty-read最终源码r3保留严格单条×50+50完成诊断及Run/effects终态 | 同锚点/正文一次提交/保存内容及真实Run未终态→完成有实际PTY证明，非全部81项PTY、真实联网模型或完整Markdown idle调度验收 |
| Android最新富字段 | `/tmp/nola-public-semantics-android-20261001.log`：**BUILD SUCCESSFUL**；83类/311项，**309通过、2跳过、0失败**，含8项新增回归和真实Compose编译 | JVM debug，不是设备/JNI/release；新字段ULong，不代表旧revision/context/history cursor/AnswerSource桥全部u64无损 |

### 8.3 既有分批验收与性能证据

这些是本轮已经取得的交付证据，不因最新批次尚未通过而抹掉；同样不能替代最新源码
重验。09-28部分`/tmp`日志/xcresult目前已不在，按第9节保存的历史结果标记，不冒充
本次重新执行；仍存在的10-01日志已复核。不同批次/过滤组有重叠，不合计成唯一总数。

| 边界 | 本轮已记录的通过证据 | 证据性质 |
| --- | --- | --- |
| 配置、协议、CLI/Host | 分层依赖门禁、CLI三协议真实loopback请求、LLM统一protocol/URL全112项、Host v7来源合同、输出准入、权限恢复、记忆撤权及cancel-only实际子进程均有逐批记录 | 受控HTTP、真实生产调用者与隔离SQLite/进程，不是外网模型任务验收 |
| Mac累计回归 | 广域contracts曾**725通过/1跳过/0失败**；52项Mac/MLX、72项seed、64项sampling/memory owner等专项通过 | 历史源码时点；opt-in picker跳过不是通过；不包括最新公共语义源码 |
| iOS累计回归 | 模拟器delivery/权限生命周期**71/71**、凭据发布**102/102**、能力链**76/76**、生成准入**62/62**等分批通过 | 实际构建/模拟器、受控provider；不是完整冷恢复或设备真实模型 |
| Android累计回归 | 完整debug JVM suite曾**301通过/2跳过/0失败**（303项、82类） | 历史JUnit结果，不含当前新富字段接线、设备/JNI或release |
| 10-01公共语义修改前基线 | Domain78、Store272、StorePort3、Core395+Host115+context2、Application182+integration33、Mac32，合计**1,112项定向通过** | `/tmp/nola-checkpoint-metadata-{core,application,mac}-20261001.log`及Store/Domain当批日志；不是全workspace，也不是最新批次结果 |
| Store Text增量规模 | 真实磁盘SQLite 2/8/32MiB，Append耗时约335/1,246/4,977ms；均Close、冷重开并核验全文SHA；100次tail+range约195ms | [完整规模记录](STREAMING_SCALE_ACCEPTANCE.md)。debug、固定checkpoint、热缓存；证明该Store链路近似线性，不证明Core/传输/UI端到端线性 |
| Mac大历史读取 | 千万字符历史的有界尾部查询约55ms、响应716字节；属于上述Mac32回归 | 不是全文加载时间，更不是模型任务完成速度 |

### 8.4 剩余硬门槛与继续顺序

1. 10-01公共语义分层回归已通过：六crate lib、完整Application tests、Android完整JVM、
   Mac七类及iOS七类实际build/test。CLI完整binary、短单帧deadline、主区同锚点及
   指定相邻PTY和fifty-read最终源码已重验。完整Markdown多行idle-tail/其它
   旧syntax fallback、全部PTY/平台UI与Worker专项仍待验收；保留失败和跳过明细，
   不把平台定向过滤组宣称为全部平台测试。
2. 公开journal生产发布要覆盖Begin/internal Begin、Host response、active submission、
   显式完成、idle Task/Skill及Store recovery。当前只在测试调用publisher，不能启用
   text-only消费者。完整publication-set、多assistant/多bounded frame、稳定count/digest/
   replay基础已通过当前共享层验收；typed changed-input preflight及显式归属已接真实
   Begin/Host/active/complete，但收集结果尚未接发布。仍需changed-owner coordinator、全提交点同事务
   接线及server-issued idle/recovery source stamp。set凭证不是首次producer覆盖证明。
3. 补Task/Skill/Attempt真实adapter与依赖/移除语义。普通公开值/journal的range、
   opaque URI值域及真实manifest分块下的NoChange已整改并通过当前lib回归；这不解决
   ToolState重新序列化既有大结果的累计成本，更不等于公开传输已可切换。
4. Core私有控制checkpoint与不可变正文分离、source-owned回滚、pending request复制和
   累计事件/序列化仍需实施；全Store物理配额、旧generation/孤立凭据GC仍未闭合。
5. 原文跨页需要真实版本，不以相同长度推断同一内容；CLI/Service/全客户端journal
   transport、正文cursor、snapshot staging与terminal watermark尚未迁移。
6. iOS完整HostInvocation/稳定execution policy随begin原子持久化、可信冷恢复provider
   重建、CLI历史secret resolver、Mac冷重启恢复及Host管理本地模型的新intent目录待接。
   默认拒绝不完整恢复的gate保持，不换成当前模型或当前key。
7. 完成全端三档UI与配置联动、app级后台Run所有者、完整交互所有者和legacy adapter
   清理；最后做全workspace、全平台、截图/AX与真实联网模型长任务/压缩/恢复验收。

已有实施与分批验收应计入交付；上述缺口不应继续笼统记成“还没开始”，也不能因已完成
很多模块就省掉生产接线和当前源码的验收。整体goal保持进行中。

### 8.5 完整发布集合与借用规划（2026-10-02）

这批把单帧/单owner限制改为一个实际source对应一个不可扩展的有序publication set：
Store唯一writer在整组savepoint内写多owner、多frame、heads/versions/records及schema33
集合凭证/成员表；source key不依赖set digest。冷Replay核验整组初始cursor、顺序、
数量、canonical batch、持久record/header与该set引入的实际chunk bytes，缺失、
重排、增删或同长度损坏都拒绝，不自动补写。

Application用实际编码与原始UTF-8预算分帧，而非拒绝合法的大逻辑增量。每帧至多
128 operations、128KiB canonical batch、128 chunks/256KiB supplied正文，每chunk
至多64KiB；分帧不改变Part generation。初始Core message→admitted message的映射
通过借用canonical public view完成，8MiB Text/大JSON及metadata指针保真；foreign
Run/message/Part在head读取前拒绝。新增16项测试，不把参数化case数或重跑数当新增数。

六crate lib的r3当前源码已通过：Application222/Core396/Domain90/Kernel121/
Store287/StorePort3，**1,119通过、0失败、1规模测试默认忽略、exit0**，日志
`/tmp/nola-journal-publication-set-libs-r3-20261002.log`。默认忽略的规模测试已用独立
命令显式通过：2/8/32MiB、Close、冷重开及全文SHA一致，日志
`/tmp/nola-journal-publication-set-scale-r1-20261002.log`。
完整Application/Store/Domain `--tests` r2已通过：**122组、1,037通过、0失败、
1默认忽略、exit0**（Application474=lib222+integration252；Domain115；Store448）。
日志`/tmp/nola-journal-publication-set-integration-r2-20261002.log`，含实际依赖门禁。
首跑只有旧public Tool fixture缺title/origin/artifacts导致1失败；补真实必需字段并
加强精确source span断言后，定向1通过，再完整重跑，不放宽生产模型或内存阈值。
CLI完整重跑已通过，具体失败、整改和范围见8.6；10-01原生/Web结果不继承为
10-02的全平台验收。各suite重叠不合计。

已证明的范围是同UoW Store整组发布和Application planner/publisher隔离链路。尚未
证明首次producer覆盖全部Core可见变化；自声明count/digest不能替代typed changed-input
visitor。生产Begin/Host/active/complete、Task/Skill/Attempt、idle/recovery仍未接线。
准备数据归外层receipt事务，set内部savepoint不回滚调用前的preparations。

另发现旧inline历史range每页两遍扫描完整JSON核验review；千万字节分页fixture所在
7项suite耗时179.74s。这是计算量仍无界的实际缺口，不是32KiB输出预算失效。需在
writer边界产生版本绑定的disclosure事实并改用真正bounded backing，未在本批实施。
[规模验收记录](STREAMING_SCALE_ACCEPTANCE.md)保留扫描原因及整改门槛。

下一步先补窄turn-owner、durable section-order及attempt-successor端口，再统一
`accepted changes → canonical applied commit → exact persisted owner/cursor → plan
→ one complete set → outer commit`。新successor的owner在canonical commit后才可查；
不放宽owner gate、不扫描整个projection，不把失败降级为可选UI错误。详细合同见
[Application adapter](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)。

### 8.6 工具目录快照与会话所有权（2026-10-02）

本批CLI完整binary首跑日志`/tmp/nola-journal-publication-set-cli-r1-20261002.log`
为**1,401通过、2失败、2忽略、exit101**。两项失败的目录差异均为Skill/skill_skip，
不是模型输出语法错误。真实调用路径有两层重复读取：Web同一解析分别构建local和
host目录；单个目录又对每个spec重读进程全局Skill配置与Host manifest。可变源
在构建期间改变时，一份目录会混用不同版本。

`native_bridge.rs`实际chat bootstrap及context resolver现在一次捕获base catalog，
local/host均消费同一快照。model-test仍不读取工具源；safe-web过滤、Device候选及
egress receipt/grant不变。`contracts_execution.rs`每次目录构建只捕获一份可用性
事实并解析一次manifest，再纯过滤全部spec。下次构建仍读取新状态；执行时
`host_capability_enabled`仍立即读取当前事实，目录快照不成为授权凭证。
这次捕获的仅是可用性flags；`specs()`生成Skill参数时仍会另读global roots，
不能将它称为完整Skill资源/ID快照。model-test零读取也只指Web此入口，后续driver
和internal rules仍构建目录，不代表整个Run从未读取工具源。

原schema、deferred工具及Core deny断言保留。schema合同使用明确的启用/禁用事实，
不再假定两次读取可变源必须相等；新增5项回归覆盖单轮local/host一致性、Skill/skip
同步可用、manifest各分支、一次捕获及后续刷新。定向、完整CLI及两个binary的
验收分开记录：Web目录5项、Host availability3项、Skill schema3项均exit0；完整
binary默认并行重跑**1,408通过、0失败、2忽略、exit0（78.38s）**，日志
`/tmp/nola-journal-publication-set-cli-r2-20261002.log`。嵌套隔离子进程的1项结果
不重复计入完整binary总数；两项Worker E2E仍未执行。产品binary另行验收，
测试harness通过不等于产品更新。`cargo build -p nola_cli --bins`已exit0，
`nola`及`nola-agent-host`当前debug产物均已重建（1m05s），日志
`/tmp/nola-journal-publication-set-cli-build-r2-20261002.log`；未重启用户现有进程，
本批没有重跑PTY或联网模型。

首批快照修复当时还没有解决全局Skill执行所有权。`acquire_core_agent_host`在新Host的
`configure_skill_dirs`之前生成目录/signature，driver创建时又读取目录，而Skill
实际执行仍读取进程全局root。已有invocation binding包含skill_dirs摘要，但摘要
不能替代执行事实：另一session改变global后，原binding并不能保证用原root执行。
后续必须把Skill目录、能力曝光及实际执行资源归到同一个Runtime/Session事实源，
并将它接入目录、Host signature、driver和Skill执行；不能以全局锁或测试串行化
掩盖跨会话污染。稳定曝光也不等于永久授权，撤权仍须在具体执行边界复验。
只读审查还定位到`tools/parts/context.rs`的Skill加载会重读global roots；A/B使用
不同根目录、相同Skill ID时可能加载另一Run的来源。这是源码可达性判断，尚未
运行真实竞争验收。cutover须覆盖schema/prompt/search、后台任务、delegated执行
和permission resume，不能在这些路径重新读取ambient global。
Runtime所有权后续整改及其验收状态见8.8，不沿用首批snapshot结果证明新源码通过。

### 8.7 下一批生产提交接线边界（2026-10-02）

只读核对真实调用者后，journal下一步不是直接打开consumer gate。Begin位于
`assistant_turn.rs`及Store两个admission入口，Host/active submission/explicit
completion分别走Application `lib.rs`的真实transition提交；它们需要共用一个
权威变化收集与publication coordinator，替换各自提交后直接返回的独立路径，
canonical writer和已有前置校验仍保持唯一。
coordinator必须由接受的typed变化确定全集，不能用提交方自声明count证明完整。

最小缺失端口是精确turn-owner、按Part主键的owner、持久section-order和attempt
successor。前两者读取同事务明确持久化的Turn→Message绑定及当前membership，
不能复用普通projection从checkpoint全Parts扫描出的映射，也不能让producer猜ID
后缀。Core事件/Store canonical mutation必须显式提供实际assistant身份。section
位置由每owner/section的持久counter分配并永久保留reservation，不能每次COUNT/MAX
扫描；attempt已有by-ID读取，successor应按已有唯一predecessor索引查询并核验
Run/effect/invocation。10-04已经实现这些端口及其真实生产绑定；当前验收与后续发现
见8.12。它们的实现不等于完整publication coordinator已经接通。

接线顺序固定为接受typed变化、canonical真正applied、读取exact persisted owner/
captured cursor/order、逐owner规划、一次完整set、outer commit、通知。保持原CAS
mutation顺序，展示层可使用最终record overlay；Task/Skill-only、前次attempt
fallback、failure清除和synthetic close都须显式纳入，不以absence表示删除。

idle当前仅保存checkpoint，recovery直接批量更新Part终态，二者都没有现有
Admission/Transition写证明。它们必须由真实writer返回提交identity及准确changed
目标，定义source-author与target-owner授权；不能把旧Run read proof变为新write
stamp。一次recovery可能涉及多个Run，不能未定义授权就放宽当前set的same-Run gate。

外层事务也不能无条件包住bare `Runtime.execute`：当前
`supervisor.has_command_unit_of_work()`会强制persist，直接先begin将消灭64KiB
hot batching，并让no-save短delta落库。需要区分receipt acknowledgement与durable
flush目的，复用同一Store/overlay的rollback所有者，但只有前者强制逐事件持久化；
纯hot变化不发布journal。真实Begin/Host/submission、idle/recovery、拒绝/replay、
successor、Task/Skill、取消及整组故障回滚通过前，consumer继续关闭。

### 8.8 Skill 的 Runtime 所有权整改（2026-10-02）

本次继续根因整改：用独立`skill_runtime.rs`保存Runtime不可变的Skill源选择，
`None`是managed默认、`Some([])`明确禁用、`Some(roots)`只使用这些显式目录。
配置值可以跨worker及权限恢复clone，但不是权限grant，也不是文件内容不可变证明。
tools层显式消费这份context，删除进程全局Skill根的读写入口；目录/schema、
prompt、preflight、实际加载和搜索必须同源。原Host manifest仍在执行时即时核验。

Host acquire在捕获目录/signature之前绑定本Runtime事实，driver及内部规则继承，
后台任务/permission resume不重新读取ambient roots。Web每Turn先取fresh settings，
再绑定同一份Skill源计算目录和创建Host，避免旧目录与新运行配置不同。model-test
及safe-web明确空Skill目录；单请求不同步或重写资源。Web初次启动的初始化只允许
managed根从未建立的情况，已有目录/禁用/编辑状态不能被重启自动恢复。

元数据目录不能自动扩大通用filesystem allowed_dirs；成功激活只绑定本次实际
materialization的资源目录，不按同名managed Skill猜路径。目录检查保留typed
`PathBuf`执行身份；成功结果通过可逆资源URI传递，不从display JSON或有损路径
反解权限。Web提示词和上下文统计使用同一catalog文本，不泄露内部目录元数据。
显式根缺失、不可读、
越界或目录枚举不完整都要返回错误，不能贡献部分目录再退回默认。文件内容仍在
加载时读取，此选择对象不声称解决所有文件替换/TOCTOU或实时撤权竞争。

参考资料的OpenCode Skill服务以`InstanceState`建立实例上下文，执行器另调用
`ctx.ask`；这支持区分目录发现与执行权限，不证明它按Nola的Run模型隔离。
OpenAI Docs分别描述[harness、环境与应用层](https://developers.openai.com/api/docs/guides/agents-api/architecture)
及[工作负载隔离](https://developers.openai.com/api/docs/guides/agents-api/environments/security)。
将Skill源归到Nola Runtime是本项目的设计选择，不是API自动提供的权限保证。

该批已完成源码编译，定向Skill组61项通过，覆盖A/B同Skill ID不同根的真实
schema/prompt/加载、并行交错、clone继承、禁用/未知ID拒绝、元数据不扩权。
此前完整CLI曾为1,423通过、13失败、2忽略；13项依赖个人配置的测试已改为显式
fixture endpoint。动态模型catalog子进程也显式隔离DB与Mac support根，避免继承
另一测试DB的tombstone revision；未放宽生产CAS。2026-10-03最新完整CLI binary
重跑实际exit0：1,441通过、0失败、2忽略（包括本批新增的5项durable worker测试）。
macOS APFS拒绝非法UTF-8文件名，真实非法文件名fixture限定Linux；纯URI身份
保真测试仍在Mac运行。不能把定向绿灯当作完整CLI、真实模型或跨平台验收。

本批只切Skill资源来源。随后从Mac会话定位的`workspace_access`展示文本扩权入口
已在8.11切为typed结果与已批准资源绑定；这不证明所有能力入口都已审查完成。
公开journal生产coordinator、Core私有checkpoint正文分离、旧inline分页重复扫描
及实时撤权仍留在整体整改门槛内。

### 8.9 Rust 共享编译缓存（2026-10-03）

按照用户要求接入mr-boxington，细节及使用合同见
[RUST_BUILD_CACHE.md](../../软件架构设计/工程规范/RUST_BUILD_CACHE.md)。本机已校验官方发布包并安装
mbx 1.21.1；没有执行全局setup、迁移现有target或清理用户文件。
两个全新等价crate跨不同checkout/target的实际第二次构建出现1 cache hit，
managed targets及pruned bytes均为0。只证明跨路径缓存复用，不能外推完整Nola速度
或已释放的磁盘大小。共享辅助模块和六个主要构建入口已接线，隔离脚本27项通过，
包括wrapper/Cargo/grandchild取消隔离、邻居存活、错误PGID拒绝、统一workspace
manifest和Xcode两个phase的helper依赖。真实共享层test编译已报告106次命中，
工具估算避免的编译时间不是冷/热墙钟实测。10-03 CLI两个实际运行binary已重建
exit0（52.91s），不是仅测试harness通过。10-04 Mac runtime两产物实际重建exit0
（45.51s，484次命中）；后续关键源码及App仍须以8.11最新验收为准。手工验证曾
漏设target而触发自动迁移，已完整恢复原目录并确认managed targets/pruned均为0；
经过和防护记录于缓存合同，不把临时布局改变隐去。Xcode验收暴露的BSD install
随机临时输出、mbx长socket路径及沙盒内进程组检测已修正，32项脚本验证通过，沙盒保持开启。

### 8.10 Pi durable 的采用与执行权整改（2026-10-03）

已核对官方`earendil-works/pi/packages/durable`的实际Session、tool recovery、
scheduler gate及存储限制，固定commit而非按宣传推断。采用合同与边界见
[DURABLE_RUNTIME_DESIGN.md](../../软件架构设计/模块/05-执行与恢复/DURABLE_RUNTIME_DESIGN.md)。Nola已有双replay-safe
栅栏，不能重复建设或因参考实现的单进程模型而削弱多进程lease/权限校验。

本批已切除真实缺口：Retry/Finish的token-only授权、terminal不注销
attempt，以及worker按task ID取得registry后继身份的风险。共享Application/Store
与CLI全部worker路径一起迁移到必填typed witness及原attempt绑定的handle；
完成、重试、续租、进度使用一致执行权。过期operator/recovery仍由持久过期事实
独立授权，不增加无witness兼容入口。Application Service协议升级至27，schema仍
为33；不会连接旧合同服务，也未重启用户正在运行的App。

同时修正CLI跨进程入口：原attempt快照必须在Shell spawn/Core执行前fresh CAS，
不能用registry最新handle借权；只有已提交Finish才发布terminal sidecar/成功退出码，
旧worker清理也不能删除后继控制handle。测试补了拒绝路径及合法worker的真实执行
正对照，避免空progress校验误差造成“副作用未发生”的假绿灯。

长流真实服务验收暴露的同步冗余也已整改：每段不再重复携带未变化metadata与
running状态，保留完整Stream CAS及真实review/lifecycle/timestamp变化、事务回滚
与跨revision gap的发布基线。原2 KiB回复约2.87 MiB队列的问题已回到2 MiB预算
以内，预算未放宽；跨页同步、receiver重启、replay和canonical相等断言全部通过。

2026-10-03最新源码的五crate完整`--tests --no-fail-fast`实际exit0：147个测试目标，
1,079通过、0失败、1忽略（Application/Store/Service/Protocol/Service Client）。其中
RuntimeTask Store21项、真实IPC worker恢复3项、新增replica过滤8项均通过。
旧service fixture的source_host_id、完整typed Tool/Reasoning及revision rejection
回执已按正式合同修正，未给生产字段添加default或放宽CAS/预算。完整CLI binary
最新重跑exit0：1,441通过、0失败、2忽略。随后CLI/Web三组生产进程合同
5+2+3共10项通过，保留模型参数、会话优先级、seed=0/2及权限断言；旧persisted
seed fixture改为正式Value变体，明确execution能力，未给生产字段增加兼容default。
10-03 CLI运行产物已重建；Mac与10-04后续变更见8.11。这不是UI、真实供应商模型
或全平台重新验收。

仅本批RuntimeTask执行权与同步事实切片完成。存量permission wait入口、owned
children完成屏障、未知commit审计、Core私有checkpoint正文分离与公开journal
生产coordinator仍见8.4/8.7及durable合同，不能声称整个durable架构已完成。

### 8.11 Mac实际会话暴露的执行和显示边界

2026-10-04核对了最新五个真实Mac会话及持久Attempt、Part和请求日志，没有修改
用户数据库或启动用户App。最新会话`20F2200A-C61A-45B4-A901-149D9994FE49`
最后的“你编译下”失败并非单纯工具选择错误：模型source缺少executionCapabilities，
Mac把缺失降为全false，wire实际为tools=0/tool_choice=none，却要求模型重试原生
工具调用三次。三次Attempt及最终invalid_model_execution_claim均有持久证据。
09-21论文改写另有两个关联拒绝：已授权目标被错误要求位于cwd内，随后新目录又
被当成必须已存在的cwd。这些是配置与资源边界冲突，不靠用户文本关键词补路由。

本次改造合同和实现：

- 模型能力未声明是待配置状态，不是“不支持”的证据。Mac模型中心保留该模型并
  要求确认；提交前拒绝缺失合同，显式toolCalls=false仍允许文本对话。共享解析及
  Web同样拒绝未声明的执行准入，不从provider/model名或chat用途推断支持。
  共享ConfigureEndpoint按解析后的URL及协议身份使旧声明失效；显示名、启用状态
  和等价URL不改变能力，目录仍可读和修复，过期CAS不得部分写入。
  Swift解码和共享模型一致：四个必填布尔值；仅省略扩展字段才采用默认值，显式
  null、未知字段及矛盾声明必须拒绝。覆盖16种必填布尔组合与往返编码，不能把
  合法的四字段声明误判为无效provider响应。
- Core在所有工具提升之后按冻结模型能力生成精确Step投影；文本步骤明确说明
  不可调用工具，保留合法示例代码与已有证据。Host不再自行删除已证明的schema；
  矛盾合同在接触provider之前拒绝。纠错提示根据实际投影选择工具或文本纠错，
  不能在tool_choice=none时继续要求原生调用。独立审查仍由模型选择，其隔离的
  两条消息合同不混入执行指导。
- workspace_access只消费严格typed grant和可逆file URI，与Core已经批准的
  具体目录绑定。展示路径不能扩权；失败、deferred、错资源或附带mutation授权
  均拒绝，默认cwd与其他Runtime的资源不变。
- cwd只解析相对路径；声明的输出按当前授权根验证。Bash显式产物建立精确资源
  preimage v2，可跨已授权根，恢复前重验全部授权及路径身份。保留100文件和
  16/32MiB备份边界；只覆盖已声明产物，不承诺任意shell副作用的原子回滚。
- Host绑定精确canonical Run身份。Core接管后外层错误只记传输诊断，不制造第二
  套Run终态；空闲usage或旧Run不得取得当前Run所有权。Mac删除传输事件拼出的
  重复失败卡片，消费真实Attempt投影；内部provider命名空间不作为显示名称。

第一轮冻结源码验收已通过：Core lib397、Host合同116及Host Protocol lib38，共551项；
最新完整CLI binary为1,462通过、0失败、2忽略（77.46s）。新真实HTTP验证有/无工具
及隔离审查三条纠错路径、无schema时none及假输出不展示；真实shell
验证授权目标在cwd外创建、隔离资源备份/恢复、撤权/符号链接及取消拒绝。不是联网
真实模型或Mac视觉证明。后续共享七crate的153个测试目标实际exit0：1,317通过、
0失败、1忽略；该首轮不包含随后新增的共享端点更新回归。随后Application完整78个
测试目标478项通过，含新端点身份/CAS三项。CLI/Web真实HTTP与技能边界10项通过：
夹具显式声明能力，更换端点先验证422且不接触provider，再通过目录CAS确认新身份。
Web全量233项及定向29项通过，Svelte 0 errors/0 warnings。审计与真实模型声明parser、
缓存脚本最终合计109项Python通过（17.52s，含53项审计、24项声明、32项缓存脚本），
日志`/tmp/nola-final-script-tests-r2-20261004.log`。CLI及Host运行产物已重建exit0（36.89s）。这些批次
包含重叠suite，不能相加当成独立测试总量。Mac最新原生合同验收实际exit0：755项
XCTest执行、1跳过、0失败，另27项Swift Testing通过；产物已在实际Xcode沙盒构建。
日志`/tmp/nola-mac-contract-fixes-r6-20261004.log`。该合同层默认排除视觉和破坏性
host-e2e，不能当成完整UI或设备验收。

真实模型的本批关键链路已验收，不依赖强制启用全工具的测试开关：

- CLI默认目录两轮联网验收exit0：首轮3个实际工具调用及3个结果覆盖两个文件，
  重启同一会话后按已有证据答复，第二轮0工具调用，数据库拥有恢复checkpoint。
  日志`/tmp/nola-real-cli-key-fixes-r2-20261004.log`。
- Mac使用最新r6原生App及内嵌Host，实际read_file成功后提交精确标记，47个事件、
  0工具错误、0正文reset；数据库1个Run、1套18事件trace、2条消息。持久化审计
  通过生产Application的ReadConversationTextRange读回分块原文，1次查询21字节，
  与完成回复及界面receipt一致；不从SQL拼块、旧答案或trace缓存填充原文。
  日志`/tmp/nola-real-mac-key-fixes-r2-20261004.log`，独立审计receipt为
  `/tmp/nola-mac-real-e2e.joaIij/runtime-session-audit-r3.json`。

审计桥接只打开同一只读事务的私有SQLite backup，绝不把用户实时数据库交给可写
ApplicationRuntime。两个Rust桥接合同测试通过；Python验证了同一只读revision、
snapshot权限、UTF-8范围，并对无reader、非法归属、
未支持的JSON正文及超预算读回拒绝，不把缺失证据转成绿灯。验收fixture使用现有
配置的provider/model及凭据，能力以明确opt-in声明固定；不修改用户模型声明，不
按模型名/URL推断生产能力。没有截图/动态视觉审查，本批也未验收长任务、复杂
Markdown、交互授权或全平台真实模型，不能称所有UI/架构问题已解决。

本次不等于完成公开journal、Core恢复正文分离或完整有序Mac时间线。Mac正文与
活动仍是分离投影，未知commit、permission wait旧入口、owned children屏障及8.4
全平台硬门槛仍需实施；整体goal保持进行中。

### 8.12 初始内容、独立会话身份与历史归属（2026-10-04）

本续批继续只读核对用户实际Mac数据库，最新用户会话仍是10-02的`20F2200A…`，
没有本轮修复后的新用户会话。其模型profile仍缺显式executionCapabilities；新代码
会提前要求确认，不自动改写现有配置。旧失败同Run的两条terminal trace属于已整改
的执行/传输归属问题；不能用旧记录证明新产品生效，也不能把inline正文为空误判为
分块正文丢失。最新几次用户会话没有internal continuation、Task或Skill，相关路径
必须依靠真实生产用例的隔离回归，而非声称用户已经验收。

第一批已落地并接真实提交：

- Domain新增显式`AssistantTurnBinding`；Store schema34同事务记录实际Turn→Message
  和Part关系，不从checkpoint正文、Run ID或Message后缀回填。Admission与非admission
  的歧义规则分开；普通Text不会因不被请求的synthetic Turn歧义而失去可读性。
- internal Begin先读真实复用Message，再在writer同时比较Message、前序Run与checkpoint
  revision。错目标、旧Run或旧checkpoint不得部分创建新Run、effect、Part或binding。
- 窄规划端口按精确Turn/Part读取owner，按唯一predecessor索引读取同Run/effect/invocation
  successor，并用持久counter分配每owner/section的永久位置。历史读证明不能授权新位置。
- Begin/internal、Host、active submission及显式完成使用同一borrowed changed-input
  visitor；Core/Skill事件穷举分类，单ID查询最终Skill、保留CAS mutation顺序、显式记录
  failure清除、predecessor刷新和canonical-only Close。结果目前只用于提交前校验，
  尚未交给journal planner/publisher；不能称完整producer已启用。
- 普通Task/Skill/Attempt消费同一组单项公共语义helper，不复制累计正文或扫描历史来
  决定owner。它们仍不是权限或执行权来源。

第一批冻结源码四crate完整测试实际exit0：Application、Store、Domain、StorePort
共129个测试目标，**1,119通过、0失败、1默认忽略**，日志
`/tmp/nola-exact-owner-shared-tests-r3-20261004.log`。包括真实Begin→Host→NextTurn→
successor→冷重开、不同Core session和canonical conversation、拒绝响应零binding写入，
以及internal错误目标/Run/checkpoint的15表零变化和正确重试。首两跑为新fixture缺
trait import和显式mock方法的编译失败，已补正式合同，未增加兼容default或放宽断言。

这一批随后又发现三个真实可达缺口，正在第二次冻结及验收：

1. 09-25真实会话的三个memory Source Part只在Core checkpoint中，没有canonical
   membership。Begin必须将接受的初始PartUpdated与placeholder、effect及binding
   一起提交，而非等待模型下一次返回或依赖显示层补卡片；replica也必须保留同样事实。
2. 合法Core session与产品conversation不同，可写却被presentation/context/session等
   读取拒绝。canonical身份由Store实际row key证明，Core root/state身份须相符；二者
   分别校验，不删除foreign-session检查，也不创造可变alias猜测。
3. internal handoff复用Message后，旧Turn、Task/Skill与失败Attempt不能因当前m.run_id
   改变而消失。历史读取使用可信`canonical_message_run_owners`，保留原Run并按
   `(Run,Part)`解析Attempt；当前writer/order仍只允许当前owner，不扩展历史写权限。

第二批尚未通过的代码不能沿用上述1,119项绿灯。公开journal完整发布、idle/recovery
提交来源、Core私有正文分离、全端传输和视觉门槛仍见8.4；本节不宣称整体goal完成。

### 8.13 Pi 源码复核与关键提交边界收口（2026-10-04）

参考项目已下载到 `参考资料/pi`，固定源码
`83692682f095528f8b71652ddacff7075e36e893`。默认 Pi CLI 的 AgentSession/TUI 与
experimental `packages/durable` 是两条不同路径，不能把后者的单进程 SQLite 调度
当成默认 CLI 的保证。Durable 的关键可借鉴点是 commit → adopt → publish、未知
commit 后 poison、只重放当前与已存声明都 replay-safe 的工具，以及显式 owned child
完成屏障。Pi 不提供 Nola 所需的内置工具权限模型，不能因此删除 Nola 的多进程
lease/撤权/具体资源校验；也不能把 Nola 的 resumed-from lineage 当 owned child。

本批已经接入实际 Runtime 的部分：

- 裸 Begin、active/idle submission、cancellation、recovery 和 durable Host boundary
  由同一 canonical transaction owner 打开 durable-flush UoW；received command 使用
  receipt-acknowledgement UoW。先提交 Store，再 adopt 热状态，纯 hot delta 不开 writer。
  两种 purpose 不能共用“凡有 UoW 都强制逐 delta 持久化”的旧判断。
- COMMIT 结果未知、rollback 失败、adoption 失败使 Runtime 永久不可复用；不能从
  内存旧状态继续执行或把 unknown 当业务拒绝。receipt query 和安全 reopen 仍需要
  service/client 与实际 worker 收口，不能仅靠错误字符串声称恢复完成。
- Host 提交 checkpoint 改为直接传递 typed `CoreCheckpoint<AgentThreadState>`，删除
  typed → JSON Value → clone → typed 的重复转换。仍有完整 checkpoint clone/持久化，
  此项不是热路径线性化完成证明。
- Task/Skill/Attempt 的 borrowed journal planner 适配已实现；保留 None/空值、精确
  浮点/计数、长 Skill Title、失败正文、explicit remove 与 retired ID 不可复活。
  canonical changed-input collector 仍未交给生产 coordinator，客户端 journal rollout
  仍关闭，不把通过的叶子测试称作生产增量链路。

当前证据：Application lib 新冻结通过 **262/262**，含 supervisor 五项和真实 receipt
故障两项；Domain/Store journal filtered 通过 **39 + 44**；真实 Begin/Host/identity 三组
E2E 通过 **6 + 2 + 16**。随后 Begin 套件新增 hot-only writer-lock 对照并通过 **7/7**：
第二 SQLite connection 持有 `BEGIN IMMEDIATE` 时短 delta 仍成功、durable checkpoint
不变；释放 writer 后 terminal flush 提交两段 buffered 内容。日志分别为
`/tmp/nola-runtime-closure-all-r3-20261004.log`、
`/tmp/nola-journal-domain-store-r1-20261004.log`、
`/tmp/nola-bare-atomic-e2e-r1-20261004.log` 与
`/tmp/nola-hot-delta-uow-r2-20261004.log`。这些 suite 重叠，不能相加作独立总数。

架构缺口优先于细节优化：exact Run 的 immutable Core identity 还要接齐 Mac/CLI
解码与控制消费；permission wait 旧 loop 还要迁移到 occurrence/continuation CAS 并
匹配 heartbeat 与新 Attempt；unknown commit 必须在 actual worker 保留待对账状态，
阻止普通 lease expiry/重试接管；然后完成 source author/target owner、idle/recovery
stamp、统一 journal 生产/消费，最后删除全量 checkpoint 热链路。Markdown 间距、
长路径和控件排版属于细节；事件顺序、权限复活、滚动跟随和 busy 收口则须先服从
上述已提交状态与运行身份，不能再用显示层补偿缺失事实。

2026-10-06 补充：Store 的 command reconciliation suspension 现覆盖回执在查询后
已经提交的竞态；任务仍需完成 continuation 对账，因此不会因回执已 settled 而
留在 running。取消请求同样保留，先撤销原 Attempt 执行权并持久化
`awaiting_reconciliation`。完成通知只接受 completed/failed/cancelled，避免把
等待对账误作终态。真实 SQLite 回归覆盖 settled/absent receipt × cancelled/active
四种组合、伪造 generation 拒绝、cold reopen 与旧 lease 重试拒绝；完整
`runtime_task_e2e` **22/22** 通过，日志 `/tmp/nola-reconcile-suite-20261006.log`。
自动 continuation 恢复仍未完成，不能据此宣称恢复链路已闭合。
Service 与 SDK lib 验收分别 **14/14、30/30** 通过，包含 SQLite 持续 receipt
提交故障下仍能保存任务对账状态、同库 reopen 与原 request 查询；日志
`/tmp/nola-service-verify-20261006.log`。CLI 的状态映射同步将 suspended /
awaiting_reconciliation 投影为 Suspended，并保留非终态分类。
身份链实际验收又发现终态事务删除 invocation_config_json，与终态 exact identity
验证冲突；两条终态事务现在保留原冻结配置及 digest，继续由 admission 的敏感数据
检查约束内容。Application 恢复契约 **6/6**、turn transaction **16/16** 通过，
日志 `/tmp/nola-app-identity-20261006.log`。CLI 异 Core 身份路由与 cold restore
两项通过，原 attempt/receipt 对账身份一项通过；Mac 尚未在本轮重新验收。
权限生命周期后续验收：普通 expiry recovery 与旧 fair queue 均排除持有 pending
permission continuation 的任务，防止空租约被当作重试资格，以及旧 readiness flag
阻塞正常队列。实际任务取消入口在同一事务关闭 pending 权限申请、挂起记录和旧
Attempt；已记录的权限决定保留。`runtime_task_e2e` **23/23** 通过，包括 cold reopen、
过期恢复拒绝、旧 flag 下公平调度、故障回滚与取消后恢复拒绝，日志
`/tmp/nola-permission-lifecycle-20261006.log`。CLI permission wait 生产循环的完整
continuation / successor Attempt 迁移仍待完成，本项不等同于删除旧权限链路。
取消实现进一步收敛：普通 task cancel 与精确 permission continuation cancel 共用
同一事务函数，删除后者重复更新。精确入口先验证 suspension revision、occurrence、
continuation 与时间，再进入统一取消；权限已批准但尚未 resume 时仍可取消，并保留
原决定。上述 23 项测试追加该竞态及错误 continuation 拒绝后仍全部通过，日志
`/tmp/nola-permission-cancel-20261006.log`。
Mac 定向验收已完成：`MacAgentHostDecoderCooperativeTests` 与
`MacRemotePermissionConsumptionTests` 共 **23/23** 通过，使用隔离数据库与
architecture-qualified macOS destination；日志 `/tmp/nola-mac-identity-20261006.log`。
该结果证明这两组身份/权限消费契约，不代表完整 Mac 产品或远端 raw Core witness
链路已全部验收。
父 Core Task 增加显式 `ResumeTaskAfterPermission` submission / command：只接受
Waiting/Suspended、未取消、旧 ownership token 匹配、新 token 不同、输入水位不变
且时间不倒退的交接，纳入既有 operation ID 去重与 TaskUpdated checkpoint。
允许旧租约在权限暂停期间到期；交接后旧 token 不再有 heartbeat 权限。定向 Core
回归通过，日志 `/tmp/nola-core-handoff-20261006.log`。CLI 生产 wait 循环尚在接入，
此条仅记录已实现的必要交接契约，不能视为迁移完成。
调度统一：Store new claim、legacy resume、typed permission resume 共用一个
`fair_candidate`，删除两份重复排序 SQL。已解决的 typed permission continuation
按相同 priority/aging/project-head 顺序竞争容量，未决权限不进入可执行队列。
`runtime_task_e2e` **23/23** 通过，新增高优先级先执行、permission successor
身份变化、重复 resume 拒绝、旧 Attempt 重试拒绝的断言；日志
`/tmp/nola-permission-fair-20261006.log`。Core 公共 submission 契约 **7/7** 通过，
覆盖权限 handoff 前后 checkpoint restore、同 operation 重放不重复交接、旧 token
heartbeat 拒绝；日志 `/tmp/nola-handoff-checkpoint-20261006.log`。
权限恢复的丢失应答确认增加 `PermissionResume` 精确只读查询，参数为 task /
suspension / permission occurrence / continuation 四个身份。恢复事务的 durable event
保存当次返回的完整 grant；查询在 pinned snapshot 中验证 suspension、原 Attempt、
successor 不可变身份与该 event。返回历史已提交 grant，不读取或替换为任务当前 token，
也不豁免后续租约 CAS。Store **23/23** 通过，包括续租后 cold reopen 仍返回原 grant、
错误 continuation 拒绝；日志 `/tmp/nola-permission-proof-20261006.log`。CLI lost-ACK
消费正在接入，此查询本身不代表 cold scheduler 已恢复权限 continuation。
上述 query 接入后 Application lib **262/262**、Service lib **14/14** 通过，日志
`/tmp/nola-permission-app-20261006.log`。CLI 新执行权限状态机处于生产接线阶段，
数据库已授予新 Attempt 而本地尚未确认时必须保持 ResumePending；父 Core enqueue
成功不能代替 canonical projection 已应用的证明。
恢复查询分类已集中回 Application：PendingPermissionSuspension、
PendingCommandReconciliation、PermissionResume 均为 typed read-only，删除 Service
重复特判；避免权限状态轮询产生写回执。序列化/反序列化后的分类回归及全 lib
验收通过：Application **263/263**、Service **14/14**，日志
`/tmp/nola-permission-query-20261006.log`。
持续 SQLite receipt COMMIT 故障下，Service 对 PendingPermissionSuspension 与
PermissionResume 的真实解码/执行仍返回只读结果，新增断言通过；日志
`/tmp/nola-query-fault-20261006.log`。CLI 已落 execution authority / heartbeat
代码的 test binary 编译通过（`/tmp/nola-cli-permission-20261006.log`）；该编译
时旧 supervision wait 循环尚未替换，不能作为新权限纵向流程的运行验收。
CLI execution authority 已有真实 SQLite Store 交接回归：挂起后不执行/不续旧租约，
typed resume 后保持 ResumePending；可续新租约但仍不执行，使用原 immutable grant
显式确认才激活，旧 handle 身份不变且旧 Attempt 重试被拒绝。该定向测试通过，日志
`/tmp/nola-cli-authority-20261006.log`；它验证 worker 状态机与 Store 的组合，尚未
替代生产 permission wait 循环的端到端验收。
CLI 生产 permission wait 已迁移到 atomic typed suspension / fresh Attempt resume。
Store 新 grant 进入 ResumePending，父 Core 队列接收不等于执行完成；canonical Task
投影确认新 worker/token、水位及租约后才激活。恢复 ACK 丢失使用精确 PermissionResume
查询；父交接 ACK 或暂时读取错误保留 pending，不重发交接。旧 token-only 调用已从
该循环移除。真实生产函数纵向测试通过（实际 SQLite + Application + Core + FIFO
父命令），验证批准后须等父投影提交才返回 Allow，且原恢复证据没有被 UI event
重复污染；日志 `/tmp/nola-permission-vertical-20261006.log`。
同时修复恢复身份校验混用两种版本：Store checkpoint revision 是提交 CAS，Core
revision 必须匹配 checkpoint 内部 journal；两者允许合法不同。checkpoint 摘要和
Store revision 仍同时校验。真实 Host 重复权限中断/恢复测试复绿，CLI permission
回归 **87/87**，日志 `/tmp/nola-cli-recovery-proof-20261006.log`。cold scheduler 与
全部旧接口删除仍需后续完成，不能把热进程纵向验收当作全恢复完成。

2026-10-06 旧权限入口删除后复验：Store `runtime_task_e2e` 24/24、Service
`permission_request_cold_restart_e2e` 1/1、CLI `--bin nola permission` 88/88。
公平调度删除旧 `_nola_permission_resolved` metadata 入口，新增无有效 continuation
的旧标记不能阻塞普通队列回归。沿实际 worker 收尾发现另一个旧身份引用：权限交接后
心跳和进度已用新 attempt，终态仍捕获最初 lease。现于 pump/heartbeat 结束后从
本 worker 的 activated authority 取得收尾 lease/token；未激活时不得提交终态，
不通过 task ID 查询并借用其他 worker 身份。真实 Store 回归确认原 attempt 无法
finish、激活且续租后的 attempt 可 finish，CLI 同一 88 项再次通过。
日志：`/tmp/nola-permission-deletion-store-20261006.log`、
`/tmp/nola-permission-deletion-service-20261006.log`、
`/tmp/nola-permission-terminal-cli-20261006.log`。
此验证包含 Service 权限记录冷重启，不代表 CLI 冷启动 scheduler 已能续接：后者
仍需接通 exact continuation、父 Core handoff 与子 Run 原权限操作恢复。

2026-10-06 首次 pending 权限冷恢复接线：scheduler、worker transport、父 Core
handoff 和原 Host 权限回调已贯通，使用 exact Run resume，禁止失败后新建 Turn。
旧 Run 的决定仅对精确 Core request、工具参数与权限说明生效。grant 提交前的
只读 owner probe 拒绝存活或未知进程，复用既有 observed-delivery 判定；Service
协议升至28，避免复用缺少新合同的旧服务。Store 25/25、CLI 权限89/89、Service
持续 COMMIT 故障下恢复读取1/1通过，详见 `../../软件架构设计/模块/05-执行与恢复/DURABLE_RUNTIME_DESIGN.md`。
测试未启动真实 OS worker/模型；已 resumed grant 再次崩溃的 successor 协议仍未
实现，相关窗口不借用历史 grant，保持阻断。未完成自动 command reconciliation
续执行、统一 journal 生产发布/各端消费或 checkpoint 热路径拆分。

2026-10-06 successor 后续收口：前述“resumed grant 再次崩溃保持阻断”的阶段限制
已由真实生产协议替代。`RecoverPermissionContinuationFair` 精确验证过期 predecessor、
同一 child Run 与权限 occurrence，原子撤销/发放新 attempt，并保留不可变 grant 链。
冷恢复向 Core 仅发一条 `RecoverTaskAfterPermission`，不再堆叠可能因旧 token
被拒绝的 Waiting/Resume 命令对。Store 队列同时改为严格会话队首领取，不能跳过
另一个消费者持有的 head。协议现29。
Core9/9、CLI权限92/92、Store任务25/25、successor4/4、FIFO1/1通过；实际日志和
验收边界见 `../../软件架构设计/模块/05-执行与恢复/DURABLE_RUNTIME_DESIGN.md`。真实Service连续四次OS重启的grant保留与
旧attempt拒绝也已通过，但不等同于CLI worker/真实模型端到端验收。
下一根因已确认：原child Run已终态、Task未结算时，普通恢复的`Ok(None)`会退回
新建Turn。须统一task→exact Run持久绑定和终态reconciliation并删该fallback。
未知command结果的自动续执行、统一journal生产/消费及checkpoint热路径拆分仍未完成。

## 9. 分批实施与验证历史

下方按当时实施顺序保留成功、失败及后续修复记录，包含重叠suite；阶段状态以第8节
和最新批次为准，不把旧段落的“当前”误读为2026-10-02最新源码已通过。

整体目标尚未完成。2026-09-28 第一批已落地：

- 依赖检查使用 Cargo metadata 的真实 package identity，涵盖目标平台、可选、build
  和重命名依赖；dev dependency 不作为生产依赖。增加 Kernel 的基础设施隔离检查。
- `GenerationOptions`、`ReasoningSelection` 从旧 Core 引擎移入
  `core/nola-domain/src/generation.rs`，旧 Core 公共请求边界直接导出同一 Domain 类型，
  没有保留第二份实现。CLI 的生产请求构造继续消费这个同一类型。
- Application 的会话高级生成参数校验调用 Domain；模型配置的 budget 下限/输出上限
  校验及 effort schema 词表也使用 Domain 定义。修正保存层接受 `topK=0` 而执行层拒绝的分叉。
- 新增真实 Application command + SQLite 测试，验证非法配置不改 settings/revision/sync，
  合法的 `seed=0` 和 `presencePenalty=0` 能提交并冷启动读取。
- 修正模块合同中“一律系统选目录”的旧要求，并为本文件添加精确 Git 忽略例外。

已执行证据（当前工作树；命令返回成功）：

| 命令 | 结果 | 证明范围 |
| --- | --- | --- |
| `cargo test -p nola_domain -p nola_application --lib --quiet` | Domain 18、Application 127 项通过 | 领域及应用单元/合同，非真实模型 |
| `cargo test -p nola_domain --test workspace_dependency_architecture --quiet` | 12 项通过 | 已纳入规则的生产依赖边界；不代表所有旧客户端已经迁移 |
| `cargo test -p nola_application --test conversation_generation_contract --test model_mcp_port_e2e --quiet` | 1 + 8 项通过 | 配置拒绝无写入、冷启动读取和目录 CAS |
| `cargo check -p nola_cli --bins --quiet` | 通过 | CLI/Host 生产消费者编译接通；不证明网络/UI 行为 |

P1 仍需完成：typed ModelRoute/Capabilities/Preferences/ResolvedGeneration、一次配置
优先级解析、跨端统一三档映射、删除原始 JSON 中的过时字段、将配置快照绑定到 Turn。
上述第一批当时尚未开始P2–P6；当前进度以第6节表和下方续批记录为准。
第一批未重跑平台 UI、全量 workspace 或真实模型验收。

P1 续批：Domain 增加 `ModelProtocol`，CLI runtime 复用协议类型解析。
Web 发送链路先解析会话最终协议，再以同一个协议进行默认参数、请求参数、
reasoning 和 reasoning continuity 校验/投影。此前会话覆盖协议后，部分校验仍使用
模型中心旧协议，存在错误拒绝和错误放行；现在保留会话优先级，不以禁止覆盖规避问题。
新增双向协议切换、非法协议、显式零值和全局配置不被改变的回归。
这只是配置解析顺序收敛，不代表完整 typed resolver 或跨端迁移完成。
续批验证：`cargo test -p nola_domain --lib --quiet` 19 项通过；
`cargo test -p nola_cli --bin nola model_profile_web_tests --quiet` 28 项通过；
`cargo test -p nola_cli --bin nola runtime_config::tests --quiet` 10 项通过。
均为本地回归，不包含真实模型网络请求或 Mac UI 验收。

最终必须逐项审查 P0–P6、所有受影响平台和本节不变量，不能以新增文档或少数单测
代替整体模块化完成的证明。

### P1 并行续批：能力归属与 reasoning admission

- `ChatOutputTokenField` / `ModelExecutionCapabilities` 移入 Domain 的
  `model_capabilities` 模块；Application 导出同一类型，不维护副本。
- 新增 Application `model_generation`，集中默认选择、覆盖、显式清空、有效控制类型、
  能力声明和选中 budget 的输出上限校验。Web 实际发送链路直接调用，删除其私有解析实现。
- 配置翻译与执行准入明确分开：稀疏会话配置的旧 preference 语法先转为 typed selection，
  最终合并后再验证有效配置；不在翻译阶段猜测输出上限。
- Domain `reasoning_tiers` 定义 typed 三档及活动能力声明。Responses 使用声明集合中
  离 low/medium/high 最近的值，同距取较低；Chat 使用 low/medium/high；budget 使用
  声明的三档。Application 的 Chat/budget 默认转换已调用此实现；Responses 三档
  resolver 已有领域回归，但原生 UI 尚未接入，不视作跨端统一已完成。
- 声明的三档 budget 只要求结构合法、递增；执行准入仅要求选中档位低于本轮输出上限。
  模型编辑器仍须校验三档都适配模型默认输出限制。未选中档位不能阻塞合法低档请求。
- `responses` / `outputLimit` 即便是非活跃元数据，也必须类型合法；自动模式中的残留
  effort 列表不能授权显式 effort。会话 `topK=0` 不再静默变成 null。

本节不代表 Mac/iOS/Android 已改用共享 admission，也不代表 Turn 配置快照已全部接通。

续批验收：Domain lib 31 项（其中三档映射 8 项覆盖全部 127 个非空 effort 子集），
workspace dependency architecture 12 项，Application lib 130 项、公共
`model_generation_contract` 7 项、CLI `model_profile_web_tests` 29 项、
`session_override_translation_tests` 2 项通过，共 211 项。
测试涵盖本地 HTTP fixture 请求构造，不等同于真实供应商模型验收。

### P1 原生接入续批：只读解析查询和发送时快照

- 新增顶层 `resolve_model_reasoning` Application 查询，不通过 ModelRepository。
  输入为本轮已冻结的 declaration/protocol/output_limit/supports_reasoning，以及
  `requested` 或 `tier`；二者不能同时指定。返回 `{selection: ...}`。
- 查询复用共享 admission 和三档映射；service 的 read-only 分类及 Mac query 分类
  均显式纳入，不落 command receipt，也不在通信失败时按写命令恢复回执。
- Mac 本地主机 payload 使用该查询的结果；查询失败必须报告，不再静默本地计算。
  输出限制和上下文窗口来自冻结请求，不按模型名在运行期间重新加载。
- 移除 send 中复用编辑器 `isValid` 的执行拦截：编辑器要求所有默认档位合法，
  本轮准入只校验选中档位；否则低档合法请求会在共享解析前被客户端重复规则拒绝。
- Mac send 冻结模型中心配置与路由，避免等待目录授权后用新 route 搭配旧参数。
  此处冻结是本地运行准入边界，不等同于所有平台已持久化 Turn 配置快照。

当前证据：Application lib 132 项、公开 reasoning contract 8 项、架构门禁 12 项通过；
真实 Unix IPC 查询测试 1 项通过，验证成功/拒绝查询均无 receipt 和数据库写入。
Mac 最终增量编译与 `MacReasoningConfigurationTests` / `MacLocalRunModelRouteTests`
两组 30 项通过，0 失败/跳过；包含真实 Application 查询、Runtime payload、tier 优先、
冻结上下文、服务缺失拒绝，以及模型目录修改后快照不变。
xcresult：`Test-NolaMac-2026.09.28_2-40-57-+0800.xcresult`（本机 DerivedData/Logs/Test）。
`cargo check -p nola_cli --bins --quiet` 通过；本批共 183 项相关测试通过。
这些不代表可视 UI 验收、真实供应商推理或完整 Mac 测试集通过。

明确剩余：Mac remote-host 提交仍未携带完整本轮生成参数；本地 MLX endpoint 获取仍是
独立资源生命周期；iOS/Android/Web 的统一三档呈现、跨平台持久 Turn 快照尚未完成。

远端身份边界补充：当前 Mac remote 提交只有模型名称，未携带 host 的 profile ID；
host 因而可能绕过档案解析并回退全局 endpoint。整改必须引入 `RunGenerationIntent`，
携带 host-owned profile ID、预期配置版本、三档和稀疏生成参数，不携带凭据或把客户端
能力声明当作权限。Mac 必须从远端目录取得身份，不能复用本机 configurationID。
host 负责读取自身档案、校验 revision、解析会话/本轮优先级并冻结 resolved snapshot；
过期/非法身份明确拒绝，不能回落到全局默认。外部 execution backend 保持其独立协议。
验收需覆盖同名模型不同 endpoint、远端配置变更、客户端伪造能力、零值覆盖和重连重放。

### P1 远端配置合同实施批次

Application 新增 `RunGenerationIntent` / `RunGenerationOverrides`：
`profile_id`、`expected_revision`、`tier`、`overrides`，字段封闭，覆盖使用运行时
snake_case 参数名。scalar 缺失代表继承，显式 null scalar 拒绝；嵌套
`generation_options` 的 null 明确清除单项，0/false 保持其值。
客户端不得提交 endpoint、credential、protocol、capability 或具体 reasoning selection。

Mac 本地保存远端会话覆盖使用
`connectedHost:<base64url-no-pad host ID>:<base64url-no-pad profile ID>`，两段均为
独立 UTF-8 不透明身份，避免相同 profile ID 在不同主机上串用参数。此key不作为远端
profile ID发送；host自身会话仍按 `configured:<profile_id>` 读取。远端命名空间禁止
客户端重定义 wireProtocol/reasoning。真实SQLite冷启动测试覆盖两台主机相同档案ID隔离。

主机在接收新intent时必须读取自己档案并核对revision，冲突返回409、档案缺失返回404，
不能进入legacy全局默认路径。目录只公开profile ID/revision/display name/model name，
不包含endpoint、密钥或任意source metadata。目录/运行均绑定host identity。
外部执行器不接受Nola model intent，继续走自己的execution_model合同。

本批生产接入已落地：鉴权 settings 目录返回最小档案投影；Mac picker只显示已连接
host目录，并有刷新及远端管理入口；提交冻结档案版本和显式覆盖，不发送本机默认值。
旧版本/删除/跨host选择明确拒绝，外部执行器拒绝Nola intent；大整数seed不经Double中转。
主机在完整合并后校验生成参数，显式null清除旧seed后可执行；目录创建默认中旧local
topK/repetition字段不作为remote隐式请求参数。后续仍需将混合参数存储彻底typed拆分。

验收证据：Application lib 136项、真实SQLite配置冷启动2项、架构门禁12项通过。
Mac最终专项44项通过，0失败/跳过；含鉴权loopback HTTP目录、远端意图、同名模型跨host
身份隔离、提交快照，以及前批本地准入回归。证据
`Test-NolaMac-2026.09.28_2-49-15-+0800.xcresult`。
CLI模型档案35项、会话翻译2项通过，`cargo check -p nola_cli --bins --quiet`通过；
本批共231项相关测试。Host新intent至实际Responses HTTP fixture的请求构造也纳入CLI专项；本批没有调用真实
供应商模型，也未做完整加密远端stream端到端或可视UI验收，不宣称整个P1完成。

当前远端目录实现限定为host模型中心中已启用且允许chat的 `runtime=remote` 档案。
host管理的本地推理模型需要独立的本地route生命周期合同，不能伪装成remote档案或
退回裸模型名称。该能力尚未接入新intent目录，属于本轮后的明确缺口；完整远端模型
覆盖验收必须包含它，不能把“云端档案路径通过”扩称全部远端运行通过。

### 本地运行合同与 P2 模块边界续批

本地模型审查确认三个不同身份不能混用：canonical profile revision、artifact revision、
实际运行的进程实例。现有本地档案没有显式tool/vision/reasoning能力元数据，不能从
GGUF扩展名或模型名称补出这些能力。持久ModelRuntime只是状态投影，不是存活证据。

先修运行生命周期：同model ID不代表可复用；必须比较实际启动的binary、artifact路径
和engine参数，运行实例使用独立ID，进程状态查询失败不能视为存活。参数保存不主动
终止任务，用户明确start时才根据启动配置决定复用或重启。下一步再把精确profile、
artifact和live runtime绑定到本地route，之后才可接入远端目录。

P2 开始把 `cli/src/llm.rs` 的三处文本include拆成真正私有子模块，外部API由facade
显式导出，兄弟模块使用明确helper进口/出口；不以全pub或生产glob import掩盖边界。
本批保留transport/controller与provider helper的历史交叉依赖，后续仍需进一步拆出
typed transport和协议适配器，因此此次模块私有化不等同于P2完成。

本地生命周期已接入 `LocalModelLaunchSnapshot` / `LocalModelLiveSnapshot`：复用比较
canonical binary/artifact路径、调用子命令和全部实际engine参数，运行实例ID每次启动
重新生成。`running_snapshot`在锁内检查真实Child，旧`running_endpoint`消费同一快照。
reuse不再先写starting；显式start需要新进程时才更新启动状态。未启动真实模型的
隔离Child测试验证存活/退出、精确身份、配置变更和实例隔离。

LLM测试按实现归属拆到 `streaming_contract_tests.rs` 及provider自己的测试模块，
不为跨模块测试而扩大私有helper权限。新增门禁检查facade的三个private mod声明和
生产文件无文本include；该门禁不证明依赖无环。下一步明确拆出contracts/transport
中立模块，形成provider依赖叶模块、orchestrator单向调用provider的关系。

本批验证：CLI `llm::` 101项、`local_models::tests` 9项、Domain架构门禁13项通过，
共123项；两个CLI binary `cargo check`通过。没有启动真实本地模型、下载模型或改生产数据库。
首次LLM回归暴露取消夹具永久等待accept：取消发生在client建连前，server却无界join。
经进程栈与listener状态确认死锁后终止该测试进程，改为无代理、有界accept/read、收到
真实HTTP请求才触发取消；重新运行完整101项通过（2.18秒）。取消产品逻辑未修改。

本批不证明：artifact同路径内容更新、模型能力声明、canonical registry与live instance
完整绑定、Host本地模型intent目录，以及typed transport单向依赖。上述仍是明确后续工作。

### P2 中立合同与生产依赖解环续批

本续批解除上一批保留的生产反向依赖，新增四个私有中立模块：
`contracts` 管共享配置、事件、结果和错误形状；`transport` 管HTTP、DNS、超时、
取消与SSE等待；`stream_state` 管流式累积、响应身份及重复输出检测；
`diagnostics` 管诊断输出，provider标签由调用方提供，不自行调用编排策略。

生产依赖方向已经收敛为：

- `contracts`、`transport` 不依赖其他LLM子模块。
- `stream_state`、`diagnostics` 只依赖 `contracts`。
- `provider_protocols` 依赖 `contracts`、`stream_state`、`diagnostics`。
- `responses` 依赖上述三个中立模块及 `transport`。
- `streaming` 编排上述模块；provider不再回调或导入 `streaming`。

重试分类实现留在编排层，不随错误数据形状迁入合同层；facade显式重导原外部API。
这解除模块依赖环，但不代表全部传输接口已完成typed设计，或重试、协议、运行准入的
所有职责已拆分完毕。P2仍在进行中。

新增Domain集成门禁使用Rust语法树检查实际生产模块：处理分组导入、别名及限定路径，
拒绝通过父facade导出绕过边界，检查中立模块/provider的允许依赖和全图无环。
测试模块被排除，但测试声明之后的生产代码继续检查；`pub(super)`可见性不误计为依赖。
门禁只证明已登记模块的语法级依赖，不展开任意宏，也不证明类型方法调用的全部语义依赖；
仍须配合编译、职责审查与协议回归。

本批验证：新增语法门禁2项、原依赖架构门禁13项、CLI LLM回归101项通过，共116项；
两个CLI binary的 `cargo check` 通过。本批没有发送真实供应商模型请求，HTTP fixture
通过不代替真实模型、端到端会话或UI验收。本地能力声明、live route完整绑定及Host
本地模型目录缺口仍未解决。

只读职责复核还确认两个保留项：`contracts::effective_max_output_tokens` 仍调用旧Core
预算策略，合同模块尚非纯DTO；`diagnostics` 与外部 `interaction` 的执行上下文/权限
日志存在跨模块相互引用。本批门禁范围是LLM子模块图，不证明整个Host依赖无环，后续
应将执行诊断上下文抽为独立合同，不能把当前通过扩称为全仓库架构解环。

### 执行上下文与输出预算合同续批

执行控制必须区分两类职责：取消标记、祖先取消传播、执行ID和线程内scope属于无UI的
`execution_context`；键盘监听、composer草稿、steering持久化和权限面板属于交互适配层。
LLM只取得前者，不通过取消对象反向调用终端或权限。调用方负责建立输入监听生命周期，
因此迁移必须核对所有stream入口，而不只是删除provider内部的listener调用。
子执行可单独取消，父取消向后代传播，兄弟之间不传播；线程scope退出恢复原执行身份。

输出上限属于冻结请求事实，不等同于上下文压缩的输出预留量。当前旧Core预算函数包含
默认值和上限clamp，复用到provider会把已准入64改成256、64,000改成32,768；后者还会
令已准入的Anthropic 40,000 thinking budget在发送时失效。整改约束：三种provider使用
已准入的实际输出上限，缺失默认由准入统一决定；无效显式值明确拒绝，不静默改写。
上下文预留策略暂保留原作用域，不借此次修复改变压缩算法。验证需检查实际请求JSON
及budget/output关系，不能仅验证一个helper返回值。

本批实现：新增只依赖std的 `ExecutionContext`，interaction中的rich cancellation
组合该token；LLM、传输及诊断只取得纯token，effect取消注册表同样保存纯token。
终端监听保留在init和Core run/resume外层，不随每次provider调用重建。
scope guard设为不可跨线程移动，防止在其他线程析构污染线程局部执行身份。
新叶模块可用 `rustc --test` 单独编译，8项取消/线程scope合同测试通过；架构门禁
新增检查LLM不得依赖interaction/terminal UI，以及execution_context不得依赖Host兄弟模块。

Domain新增 `resolve_output_token_limit`，缺省16,384，显式64..1,000,000原值保留，
零值和超界值拒绝。Application准入、Host绑定、runtime档案及provider校验复用它。
Core invocation plan不再调用上下文预留策略，Host runtime不再把预留量注入模型档案；
旧预算函数更名为 `context_output_reservation_tokens`，限定压缩规划用途。
预算回归包括三协议真实loopback HTTP请求的64/64,000（6次），以及Anthropic
40,000 thinking / 64,000 output（1次）；非法零值在建立连接前拒绝。

当前通过证据：Core lib全388项、Application lib全136项、LLM全105项；Domain输出
准入1项、架构门禁17项，以及Host档案/绑定输出一致性1项。CLI execution_context
8项、interaction19项、context_budget26项、Host driver137项、cancellation15项通过。
两个CLI binary编译通过。取消测试中的MCP loopback受本机系统HTTP代理干扰，限定测试
进程设置 `NO_PROXY=127.0.0.1,localhost` 后15项通过，未修改系统设置或生产代理策略。
未调用真实供应商模型，不以loopback测试代替真实模型会话验收。

收尾还修复Emitter原生输出失败的取消归属：优先取消显式绑定的执行，而不是仅依赖TLS，
没有绑定时才使用当前Host scope。新增无线程scope的BrokenPipe合同和无关ambient任务
隔离测试；原生trace transport专项16项通过。这是失败处理边界测试，不声称已验证
真实Mac进程断连、窗口交互或所有输出传输故障。

仍未完成：provider探测到的已知输出cap尚未统一进入准入拒绝合同；上下文预留仍是原
启发式算法，未来必须验证真实输入加输出是否匹配模型窗口。此次移除静默覆盖不等同于
整个预算解析器或P1/P2/P6完成。

### 已知能力上限与冻结准入快照续批

输出准入分为 `OutputTokenPreference`（Default/Explicit）、`OutputTokenLimits`
（已知cap或unknown）及 `ResolvedOutputLimit`（tokens与来源）。显式值只允许精确保留
或拒绝；默认值取产品默认与已知可执行候选cap中的较小者，记录能力默认来源。
cap为0或小于产品最小值是无效能力事实，不能降格为unknown。没有cap不按模型名称猜测。

Host在建立新Run前收集可执行候选的能力事实，冻结为typed `HostOutputAdmission`。
每条事实携带 `HostModelRouteBinding`：精确provider、端点、模型及凭据摘要；query只保存
摘要，不存可复用密钥。不可执行fallback不参与，但其判定必须与真正provider候选策略
共用。恢复旧Run消费持久快照，不重新probe；当前route/凭据变化仍明确拒绝恢复。
这里的身份校验针对实际恢复执行器：cold恢复须提供与持久快照相同的route/凭据；warm
恢复继续使用已持有的原Run执行配置，不应因用户改了下一轮的全局默认而误拒绝原Run。

HostInvocationBinding版本升级到2，新增必需的typed output_admission，不用自由JSON
藏入关键合同。协议校验主route、fallback顺序子集、重复route、默认/显式来源以及
generation.max_output_tokens与Domain解析结果一致；缺失快照的旧版本不能冒充已准入。
CLI新提交的无效偏好必须早于Run事务拒绝；能力探测可读但不能先写会话、提交用户消息
或消费恢复状态再返回配置错误。

effects可以进一步收窄模型行为，但不能扩充已准入route集合：清除reasoning的协议
响应/压缩请求不能重新激活未准入fallback；primary不可用时也不能提升未准入目标。
默认output因已知cap降低后，reasoning budget必须按最终值重新校验，仍在提交前拒绝
冲突；内部summary目标只能在本轮已准入output范围内缩小。

metadata解析还需区分上下文估算与准入证据：output cap只归属于精确model id，不从
大小写折叠、别名或空模型命中的任意首项继承。只包含output cap的记录同样有效；旧
上下文匹配可留作估算，但不能因此产生“已知输出能力”。

本批已落地 `host_output_admission` 独立模块：新提交先准入再进入Run事务，warm恢复
沿用live Host原执行配置，cold恢复从checkpoint的typed binding还原并校验身份；
continue-last先按已有会话选择规则确定身份，避免先probe后才选中旧Run。RuntimeConfig
拒绝反序列化外部提供的output_admission，新提交清除上一轮process缓存。实际LLM
fallback列表由准入快照生成，effect清除reasoning也不能扩大候选集合。

探测缓存以结构化身份tuple的SHA256为键，保留模型、URL path/query大小写与字段边界；
provider协议不再从进程环境回读覆盖。三协议的output-only、0及小于512的cap均保留
为原始事实，由Domain决定合法性，不使用旧context-window范围过滤。内部summary只
缩小已准入output；初始化生成用独立实际请求意图准入，不校验它不会执行的fallback。

本批验证：Domain lib32项及新增输出准入合同8项；HostProtocol全23项（含6项快照
身份/伪造/版本测试）；Core全388项；Application全136项；架构门禁17项。
CLI输出准入7项、Host driver139项、LLM105项、runtime45项、context34项通过。
runtime专项包含真实SQLite权限恢复，以及cold路由改变后零HTTP请求、checkpoint与
outbox不变；metadata专项用真实loopback HTTP验证默认解析与显式超限拒绝。
CLI双binary编译通过。涉及loopback的测试进程设置NO_PROXY，未改系统代理或生产策略。

本批不证明：所有provider的实时能力目录覆盖、所有端配置快照统一、真实供应商模型
运行、Mac UI恢复显示，或完整输入加输出的窗口准入。input/total字段目前仍供上下文
规划，可能包含legacy hint，不把它们宣传为已完成的严格窗口能力合同。

### P4 实施切口复核

只读审查确认Mac的 `traceReducer.apply(projectsContent:false)` 已禁止Trace写正文，
不能把旧问题假设继续当作现状。当前主要混合点是 `MacWorkspaceModel` 同时拥有运行
状态、投影订阅、paced assembler及终结交接。拆分顺序为：

1. `MacRunCoordinator` 独占generation、运行任务、取消/恢复与typed终态。
2. `MacProjectionObservationCoordinator` 独占订阅任务与交接，复用WindowModel分页。
3. `MacAssistantPresentationSession` 收拢正文接受/展示位置及Markdown呈现状态。
4. 不可变 `MacRunSubmissionSnapshot` 在首次await前冻结配置/附件/权限与身份。
5. 交互所有者用绑定run/request/call/nonce的单一challenge代替零散Optional。

优先修复证据：远端service正确拒绝非终态EOF，但Workspace重放失败/恢复截止后仍可
调用 `finishAssistantStream()`，从而标记完成、增加terminal revision并清除running。
Coordinator必须区分confirmedTerminal、interrupted、awaitingRecovery与cancelRequested。
删除门槛是Workspace真正失去这些决定权，而非仅把原函数搬进extension。此节是明确
实施计划；以下记录首批实现，不能等同于五步全部完成或UI验收。

### P4 运行终态所有权首批实现

新增 `MacRunCoordinator`，接管运行任务、generation、取消意图及已确认终态。
首次远端执行、重放与主动恢复采用同一完成门槛：匹配会话及Run身份的canonical
终态投影成功提交后，才允许结束回答呈现。EOF、重放批次结束、Core Turn终态和
取消HTTP成功均不是该证据。等待恢复时拒绝重复提交，传输错误不写入模型正文。

进一步检查发现，`projectsContent:false` 并未禁止Trace reducer的生命周期副作用。
因此远端 `run.completed/failed/cancelled` Trace在进入reducer前被隔离，不能借由
呈现生命周期绕过canonical终态门槛。取消请求期间保留权限challenge及服务receipt；
取消失败恢复原交互状态，期间若已断连则回到等待恢复，而不是错误回到运行中。

首轮Xcode专项执行215项，1项跳过、0失败：包含11项真实Coordinator状态机测试、
首次send后正常/异常EOF不完成且不重复提交的Workspace测试，以及3项真实loopback
HTTP取消receipt测试。跳过项是需显式启用的真实文件夹picker取消测试，并非生命周期
回归。测试注入readiness与隔离配置，不修改真实用户凭据或批准权限。
这不是供应商真实模型会话或截图UI验收。后续修改仍需再次集成测试。

本批边界：本地Run终态策略尚未统一；projection订阅与assembler仍由Workspace拥有，
不可变提交快照和challenge所有者拆分尚未完成。取消等待期间恢复按钮的交互及迟到
canonical投影的状态交接仍需专项验证，不能将本批标记为P4完成。

订阅恢复的补充合同：`activeRun == nil` 仅表示当前投影没有活动Run，不足以证明某个
指定Run完成。订阅需读取绑定conversationID/runID的持久终态证据，与重放共用确认
入口；迟到终态同样必须清理恢复状态，不能让已经结束的Run永久阻塞下一次提交。
取消已接受但终态未确认时，恢复交互只能重新观察状态，不能重新发起执行或清除取消
意图；没有可用观察身份时应明确说明等待确认，不能提供点击无效的按钮。

提交快照的后续验证点：send当前已冻结多数模型、协议、档位、权限及附件字段，但
Task闭包仍读取selectedPromptTemplateID及偏好来源。后续快照拆分须在调度Task前
固定请求意图与所用模板内容，异步取得的上下文按同一会话身份组合；不能以闭包里的
零散let变量代替完整、可测试的提交合同。

### P4 订阅所有权与迟到终态续批（验证中）

`MacProjectionObservationCoordinator` 接管按会话分组的订阅Task及lease。替换任务先
取消并等待旧观察者结束；取消仍保留句柄供后续交接等待，旧lease完成不能清除新任务。
Workspace不再直接维护订阅Task字典。观察者自己触发终态交接时传递lease，避免等待
自己的Task；其他完成入口必须cancelAndDrain。终结由RunCoordinator单次认领。

Application新增只读canonical_run_terminal查询，通过Store精确查询conversation/run
身份及持久终态，不以活动Run缺失作为完成依据。订阅、重放共用该证据路径。取消待
确认时有trace则只读检查，无trace明确禁用恢复按钮；不重发执行、取消或更换generation。
首轮集成因新增测试误用target名作为Swift模块名而编译失败；修正为真实模块Nola后，
统一Xcode专项226项执行、1项opt-in picker跳过、0失败。包含4项订阅所有权测试、
6项真实HTTP receipt测试、16项远端权限/恢复消费测试；Store精确终态查询和Application
只读查询/remote session合同另4项通过。未调用真实供应商模型，不代表完整UI验收。

receipt现由Run与流实例共同绑定，旧流的迟到终态不能清理新Run的权限凭据；interrupted
重放不触发终态清理。stop不再提前释放已在途权限请求的busy状态，取消失败后也不能
重复提交同一决策。

专项通过后只读审查仍发现完成所有权缺口：idle订阅会重复消费旧终态而自行退出；
竞争失败的完成入口可能提前解锁输入。正在补明确的finalization状态与调用方交接测试，
226项绿灯不作为这些新增竞态已解决的证明。

完成交接进一步分开“持久Run已终结”和“回答呈现已收口”。`FinishDisposition`区分
completed/alreadyCompleted/inProgress/stale：订阅竞争失败立即退出供完成者drain，
网络流竞争失败则等待唯一完成者，不提前解锁输入。新generation释放旧等待者；每个
呈现阶段await之后重新校验generation。isFinalized阻止idle订阅重复终结同一Run。
这一续批正在重新运行集成测试，前述226项不包含此次完成交接修改的验证结论。

另确认窗口生命周期缺口：订阅闭包中的weak self在进入无限观察循环后变成持续强持有，
而窗口owner没有detach。下一步增加展示层attach/detach：停止并等待UI订阅退出，重新
展示时恢复观察且保留分页。关闭窗口不等于用户取消Run，不能将UI析构映射为cancelRun。
仅加coordinator.deinit不能打破Workspace—Task循环；需真实weak释放及不发取消的测试。

完成交接续批首次Xcode结果：229项执行，1项picker跳过，2项失败。两项失败均发生在
完成后真实rename：Store报告无published content base，不能发布metadata mutation。
追踪确认远端begin/terminal commit没有本地路径已有的同步发布步骤；并非projection
fixture缺少某个应由Host提供的字段。同步基线属于本地Store事务责任，不能要求UI或
测试伪造。修复需在远端准入/提交事务内沿用首次content发布、后续append/transition
delta合同，同时保持rename/settings只发布metadata；不以全量历史重发掩盖缺口。

远端终态还有一项协议差异：其owned assistant Parts是精确替换，不能简单发布Upsert，
否则副本残留placeholder或上一版本Part。新增typed `ReplicaAssistantReplacement`，
绑定message/run并列出该消息完整Parts；Kernel校验身份非空、重复项、连续ordinal及
与普通mutation/declaration冲突，Store核对既有assistant的conversation/run归属后
原子替换。其他消息的Part ID冲突必须让整个接收事务回滚。这个协议只替换本轮拥有的
assistant，不允许远端历史窗口覆盖本地既有历史。新增source→replica回归验证同步顺序、
替换残留、历史不变、重放幂等及伪造归属回滚；执行结果待下节记录。
引用复核还发现旧 `commit_remote_transition` 只剩Store定义及ledger测试调用，未通过
Store Port/Application接入生产。此次不把它计入已统一路径；后续应删除未接线的旧
协议及只为其存在的测试，而非补成第二套兼容事务。

展示生命周期实现边界（待集成验证）：owner显式attach/detach，idle订阅退出；未完成Run
暂保留一个只查询持久终态的观察者，不刷新分页或逐字呈现，终态后释放。重新attach
发布并刷新原window，不重置历史分页。Coordinator取消保留旧lease以供重挂载drain，
旧批次drain只能释放所捕获的lease。这是过渡实现：后台Run最终仍须迁移到app级运行
所有者，不能把未完成任务继续持有Workspace称为完整运行/界面生命周期分离。

远端同步Rust验证：新增remote_content_sync 4项通过（含真实源→副本rename/settings、
历史保持、幂等、非法owner及跨assistant Part冲突回滚）；append_messages 7项、
conversation_metadata_sync 7项、turn_append_sync 5项、ledger 15项、sync_replica 1项、
Kernel conversation_transition 5项通过。Mac包含该修复的统一构建/专项仍在进行。

提交快照只读复核收窄了真正的剩余工作：model/route/effort/permissionMode/附件已在
send同步捕获，不应重复重写。仍需固定的是Task调度后读取的模板/用户偏好、platform
跨await读取的启用选项、web sources选择、workspace及bookmark grants、external session
身份，以及remote执行前重读的会话/消息对象。准备上下文后的Host调用还缺generation
与cancel intent门禁；首次事件才检查已太晚，准备期间停止不能随后偷偷创建Host Run。

包含同步修复的第一次Mac复跑：234项执行，1项跳过，9个断言失败。继续定位而未放宽
测试，得到三个独立缺口：remote begin成功后没有更新Mac的isPersisted，导致完成后的
订阅门禁拒绝；Application远端投影保留Host旧title而非本地canonical header；首次
attach因默认attached=true而被短路。分别修在真实持久化成功边界、Application投影
组合边界及展示生命周期入口。Application新增完整/窗口/分页三种读口的连续rename
回归，canonical消息内容不变；历史Part引用中的canonical revision合法随metadata
revision推进，不将其误当作正文被改写。上述修复正在再次统一验证。

最终续批验收：修复后统一Xcode执行234项、1项opt-in picker跳过、0失败，包含
RunCoordinator 13项、ProjectionObservationCoordinator 6项、remote permission消费17项、
HTTP receipt 6项、presentation lifecycle 3项，以及原产品路径/Workspace行为测试。
两次idle rename、stream/订阅同时终态、idle/active终态后weak释放、历史分页保留及
重新挂载继续观察均通过。Application remote_projection_identity 4项通过（新增三种
读口连续rename断言）；共享层架构门禁17项复跑通过。此前失败记录保留，用以解释
为什么不能只靠低层单测或最初226项绿灯宣称此批完成。

此证据仍不是完整P4验收：提交快照/准备期间取消、呈现session与challenge所有者拆分、
app级后台Run所有者、真实供应商模型和截图/AX交互验收仍未完成。

### P4 请求快照与实际提交门闩（实施及验证中）

`MacRunSubmissionSnapshot` 同步固定请求消息、模板/偏好组件、原draft/附件、平台与Web
来源选择、工作区选择、外部session及远端连接。已有模型route/参数仍沿用send同步捕获，
不重新解释三档或能力。异步prepare只解析这些选择；平台OS授权、来源blocked/删除及
目录授权撤销实时重验，新增授权不会扩大旧请求。Web请求返回自身fetch结果，并只在
source仍与原值匹配时更新缓存，防止编辑URL后旧请求覆写新来源。

线程安全一次性 `MacRunSubmissionTicket` 固定conversation/submission/generation身份，
preparing与claimDispatch原子竞争。准备中停止获胜则恢复输入/附件、撤回临时展示，不
发Host cancel、不伪造canonical cancelled；旧准备任务迟到也不能消费门闩。local在
process.run前、remote在HTTP前消费门闩，而不是仅在Workspace调用service之前检查。
dispatching表示可能提交；只有匹配的接受证据才能变成accepted并发送此前暂存的取消。
断线不能降格为未提交。只有适配器明确知道尚未调用transport的准备/镜像失败可记为
rejectedBeforeDispatch。该状态不等同于持久Run终态。

remote在claim后、HTTP前通过MainActor callback提交冻结消息的本地镜像，镜像失败不
发送HTTP。连接target在send冻结，已开始Run的恢复、取消、权限及任务控制沿用该target，
不能因为修改下一轮默认连接而发往另一个Host。准备被放弃时不替换活动权限receipt。

新增验证包括：门闩跨Task竞争只有一个赢家、准备停止后run/cancel均零、模型上下文和
外部session冻结、目录授权撤销、新来源编辑/撤销、HTTP settings暂停期间停止、镜像
失败零HTTP、切默认Host后原Run仍发往原Host。Swift parse已通过，统一Xcode正在执行；
不能把这些新增测试的存在当作通过证据。

本批边界：远端连接target是进程内执行快照，不序列化凭据；冷启动恢复仍需按持久Host
身份重新绑定配置，不能宣称内存target已解决跨重启恢复。local进程启动成功但初始请求
写入失败的存活/回收证据也仍沿用既有本地策略，不能据此宣称local终态协议整体完成。

本批验证结果：首次统一Xcode 250项执行、1项跳过、1项失败，原因是新增测试缺少
canonical会话，设置会话级平台开关被Store正确拒绝。补typed会话创建与真实持久化
断言；同时为平台服务注入UserDefaults，测试仅使用独立suite，不改用户偏好。再补
“取消新请求准备不得清掉旧Run receipt”的真实HTTP回归后，统一Xcode 251项执行、
1项opt-in picker跳过、0失败。包含10项远端HTTP合同、5项门闩、2项Workspace准备
流程、2项目录授权快照、4项上下文选择快照，以及前批终态/窗口/产品路径回归。
已有fake runtime也按实际claim/accept/callback合同接入，并未跳过门闩以维持旧测试。
真实供应商、冷启动连接重绑定及全端接入仍不能由这批专项结果证明。

扩大范围的 Mac contracts 已完成：620项执行，1项跳过，22个失败断言（含1个
unexpected），因此不能将251项专项通过视为全局验收。失败集中于两个数据源测试的
会话级覆盖持久化，以及两个本地化测试的文案覆盖。当前并行分工：一条核实canonical
会话前置条件并修复数据源回归；一条对齐产品文案与翻译目录；一条只读审查呈现状态
与运行控制的所有权边界。统一Xcode集成验证由主代理串行执行，避免并行构建争用。

### P4 呈现会话所有权（实施中，尚未验收）

只读追踪确认：canonical文本先记入accepted，再由pacing buffer分段并取出；reveal在
detach/cancel后退出时，尚未显示的segments仅在已退出调用栈中。重挂后同一canonical
文本被判unchanged，因此不会补齐。这不是调整刷新频率能修复的问题，而是“已接收”
与“已显示”之间缺少持久的呈现队列所有者。

引入MacAssistantPresentationSession，统一拥有accepted text、visible text、parser、
pacing buffer及待显示队列；Workspace只提交输入并应用呈现frame。暂停不消费未显示
内容，同值快照也可恢复队列；非前缀替换废弃旧reveal lease，防止旧异步循环污染新
正文。RunCoordinator继续只管理执行身份/接受/取消/终态，不拥有Markdown解析器。
session单测必须覆盖暂停后无新delta、无terminal也能补齐，以及替换使旧lease失效。
Mac远端写入口只允许terminal projection，运行中trace/CorePart消费也不直接更新正文；
因此不能用running状态伪装terminal envelope以制造“真实非终态”验收。Workspace层
以合法canonical终态长正文验证detach/attach不重发Run、不触发取消、不重复正文；
运行中远端正文同步入口的缺失需单独处理，不能由session单测代替端到端证明。

广域失败修复已落地、待统一Xcode复测：数据源fixture现在先typed创建真实会话，保留
unknown conversation拒绝不变，并将原先会误共享/tmp数据库的两个测试改为独立目录。
本地化补12项键的所有支持语言，旧Model Center断言对齐当前产品API request parameters。
共享层依赖及LLM模块架构门禁17项复跑通过；这不是Mac运行测试通过的替代证据。

本轮统一Mac contracts已完成：626项执行，1项opt-in picker跳过，0失败。新增
MacAssistantPresentationSessionTests 5项以及Workspace终态长正文detach/attach回归
通过；先前数据源和本地化失败也已消除。日志为本地临时文件
`/tmp/nola-mac-contracts.TrpEt2`，不提交日志。该层排除了visual、host-e2e与真实本地
模型测试，不能据此宣称完整UI或真实供应商验收。

### P4 远端运行中正文同步（合同与实现中）

确认的断点：Host运行中发CorePart/session事件，但Mac正文只读取canonical订阅；
现有remote commit又只允许terminal projection，所以运行中的本地正文没有更新来源。
不恢复Trace拼正文，也不放宽终态合同允许running。新增独立的已提交assistant投影合同：

- 源端从真实Application提交后的read model获取当前Run拥有的assistant及Parts，只传
  此消息，不重复传历史；revision/cursor必须来自该次已提交状态。
- 目标Application/Store验证source Host、conversation、submission/Run、assistant
  所有权及版本；同版本同内容幂等、同版本异内容冲突、旧版本不能覆盖新内容或终态。
- 消息Parts、source watermark、目标本地revision、projection通知及同步outbox同事务
  提交。不能借正文投影创建新Run、改用户历史、授予权限或宣告Run结束。
- Mac只提交typed合同并消费canonical订阅；权限receipt与terminal接受仍由原独立合同
  管理。新增真实运行中投影集成回归后，才能补上此前缺失的无terminal恢复证据。

此项按共享层、Host发送端、Mac接收端并行实施，接口先冻结，统一集成验证后记录结果。

身份与读取边界补充：Host绑定必须在open remote turn时由冻结的连接选择写入Run，
不能让第一条正文事件自行声明可信Host；同一绑定也必须约束现有terminal commit，
否则严格校验partial仍可被另一Host的terminal绕过。绑定是Run级而非整个会话永久
固定，新Run可切Host，旧Run的控制与正文仍属于原Host。源查询从执行该commit的同一
Application port取得一致的消息/版本，不新开默认数据库，不将Core未提交Part混作
源read model；source revision与目标local revision各自推进，不能相互替代。

恢复请求必须携带显式expected_run_id：trace.run_id不一定是canonical Run标识，不能
猜测或用当前active Run代替历史恢复的目标。通用历史trace读取未指定Run时不附加
运行中正文快照，恢复链路则必须传入预期Run并再次验证返回身份。

查询边界同样纳入实现：首个remote Run在尚无terminal projection JSON时仍必须能读
运行中正文；已有上一轮terminal snapshot时，新Run不能继承其active_run=nil或旧
revision。完整、窗口与分页读取必须以当前canonical消息/Run及local revision组合，
不能只验证Store中字符串已更新。根线程新增的存储e2e同时要求冷启动后重放幂等，
本地交付时间变化不影响source内容身份，且新Run不能抢占旧assistant的Part ID。

实施中验证：新增Store运行中镜像e2e 4项通过；Application首/后续remote运行中读取2项、
历史Port 2项及remote identity 4项通过。读取合同现在外层Message/Part revision使用
本地canonical revision，Host原始revision仍保留在不可变Part字段及source digest中；
旧断言将两种revision混为一谈，已分别验证而非删除版本断言。CLI完整编译通过，但
发送链路新增测试仍未收到预期committed事件（此前可选snapshot fixture错误已修），
不能据低层通过宣称闭环。共享层全测试和发送链路定位继续进行，Mac新链路尚未Xcode验收。

后续证据：CLI测试短增量使用InProcess热缓冲，未达到持久化边界时commit.applied=false，
不应产生committed正文帧；测试需分别验证未提交不发与真实持久化后发布。审查同时发现
新显示查询在accept成功后用`?`传播失败，会把已提交执行误报失败；且无订阅的CLI也
执行该查询。正在引入显式显示订阅、applied门禁、dirty合并及显示错误隔离。

性能边界必须保留：单槽内存队列不等于线性增量。累计assistant快照每次发送都重读/
重传/重写及进入同步outbox，仍可能累计O(n²)。本批限频只降低常数，后续必须以已提交
typed append/replace/remove、base revision/digest和有界恢复快照解决；不能退回Core
未提交delta。既存terminal完整投影读失败会丢弃待发终态也是未解决项，超过2MiB读取
预算时不能仅跳过live错误或增大限额便宣称大正文端到端可用。

共享层广域首跑Application lib 137项通过，随后数据源生命周期2项因fixture没有创建
会话失败。补typed consumer创建后该suite 4项通过；没有放宽Store外键/owner校验。
已启动no-fail-fast广域复跑以一次收集其余失败，结果尚待确认。

增量合同预研（尚未实现，不能计入完成）：Domain已有AppendText与前置digest，Store
也在提交事务中发布ConversationTransitionReplica。但pending_sync_changes受独立同步
消费者ack及全局批次限制，不可直接当远端会话订阅日志；裁剪单assistant后也不能用
全conversation source revision当连续CAS序号。需要独立per-Run/assistant已提交序列和
不可变操作记录，接收端验证base sequence、operation identity/digest及Host绑定。
原始PartMutation还包含尚未批准的review候选正文，Task/Skill/ModelAttempt又部分来自
读模型合成；因此不可直接把outbox原始delta交客户端显示，必须保留共享Application的
可见性和typed投影语义。底层AppendText目前仍重读全文算摘要且SQLite整行拼接，故即使
传输改为delta也不能宣称端到端线性；存储分段及有界读取仍需独立验证。

共享层no-fail-fast结果：112个测试目标累计680通过、3失败。失败分别为权限读取期望
遗漏已存在的receipts字段（2项），以及旧版本fixture只改user_version却遗留v27新表
（1项）。补完整receipts断言及“interrupted记录不授权”回归后permission suite 4项
通过；直接构造真实v25/v26数据库并验证26→27保留PDF/会话、空Host绑定和重开幂等后
migration/resource suite 5项通过。生产迁移和权限模型未为旧fixture放宽。

发送端订阅/读取隔离实现后CLI再次完整编译通过：4项专项中3项通过（plain无显示查询、
received事务短delta发布、9MiB查询失败不推翻commit）；最后一项在ModelCompleted后的
pending-frame断言失败，正在区分model结束与Run终态及队列清理，尚不能计全通过。

该CLI fixture进一步定位：重复3000次的段落触发既有model_output_is_structurally_repetitive
保护，Core合法派生新的synthesis，而非已完成Run。改为同等规模的不同编号内容，并用
正式AgentItem构造及request text-part身份；不修改重复保护。保留真实effect结算、
outbox为空和active Run清空断言，修订后的复跑仍待Mac构建释放编译窗口。
Mac新增运行中正文3项以及真实HTTP身份/权限receipt隔离4项已进入全范围contracts构建，
尚未获得执行结果，不能计入此前626项绿灯。

该轮Mac contracts最终未通过：xcresult记录659项，653通过、1跳过、5失败。
其中连续SSE帧在terminalThenLate测试被拼成一个JSON，另4项测试进程崩溃需独立
核对调用栈，不能假定同因。独立真实HTTP实验确认Foundation AsyncBytes.lines
不保留空行；因此不能用它承载依赖空行的SSE帧边界。整改采用字节级帧解码，
同时保留多行data、不同换行形式与流结束语义，禁止以逐行JSON解析绕过协议。

本地进程资源所有权另行收敛到MacLocalAgentProcessSession：launch成功立即登记，
首次stdin写入及后续读取失败均经同一个owner清理；退出通过Foundation回调观察，
不再竞争waitpid。新增5项真实隔离子进程测试，目前仅语法检查通过，待统一构建。
此变更不代表本地Run终态合同完成：Workspace在post-launch错误/取消时仍有直接
标记消息终态的旧分支；后续必须以canonical Run事实区分执行终态与传输中断，
不能把资源已释放等同于业务Run已完成。

并行复核进展：CLI committed_assistant_projection_tests修订fixture后4/4通过，
既有重复输出保护未改。Mac崩溃的3份精确匹配诊断栈指向decodeUnbound的巨大
同步栈帧（Thread stack size exceeded，约514000字节），与SSE分帧故障独立。
先将事件解码按family划分独立调用边界，再由原真实HTTP用例验证；不能以调大
线程栈、改超时或只让测试改用主线程来消除报错。当前MacLocalAgentEvent仍汇集
许多互斥的可选envelope，后续事件模型应表达互斥typed payload，而非继续堆字段；
本批局部拆分不得宣称已经完成这一模型迁移。

统一验证已完成：Mac contracts exit 0，654项中653通过、1项既有opt-in picker
跳过、0失败，无测试进程重启。新增SSE 12项、cooperative decoder 4项、owned
process 5项均通过，远端取消receipt的14项真实HTTP合同也通过。此前失败运行的
xcresult汇总包含重启重跑记录，不能将其659记录与本轮654唯一测试直接相减。
Store/Application广域lib及integration tests复跑exit 0：112个目标、685通过、
0失败；CLI committed-assistant专项4/4通过。日志分别为
`/tmp/nola-mac-contracts.f1RhKt`和`/tmp/nola-shared-projections.DZTY4U`。
这些是隔离合同与本地HTTP/子进程验证，不是云端真实模型、真实IPC或视觉验收。

### P4 本地传输中断与执行恢复（进行中）

进程资源owner已经独立，不代表执行生命周期已统一。Workspace原本对本地
CancellationError直接写cancelled，对其他错误/EOF直接finish；这绕过了远端已建立的
持久Run终态合同。整改要求所有执行位置经同一个精确conversation/run查询与完成
仲裁：持久completed/failed/cancelled事实优先于迟到transport错误，没有证据则保留
身份与取消意图，停止转圈但不发布消息终态、不成功通知、不重新提交原请求。
恢复卡提供只读检查；composer明确阻止覆盖未决Run，保留用户草稿。

只读检查是本批安全边界，不是最终恢复产品。共享Application已有
RecoverOrphanedRuns/RecoverObservedOrphanedRun：校验checkpoint、delivery lease、
worker generation及token，按effect策略决定Retry/Resume/OutcomeUnknown；CLI的
resume_agent_core_expected_with_cancellation恢复精确原Run和冻结配置。后续Mac
必须通过窄Application/Host恢复合同接入，不能把“重新launch相同prompt”作为恢复。
建议返回typed disposition：terminal/resumable/awaiting_permission/outcome_unknown/
still_owned/missing/blocked，并由应用级Run owner驱动，而非UI猜测。
当前observed恢复要求非空delivery，尚不能覆盖首次claim之前退出；一次查不到Run
不能证明从未提交。该窗口需要提交receipt与精确owner证据。恢复返回值目前以数量
为主，也需补足typed disposition与终态投影原子发布，才能形成完整闭环。

本批实现已进入验证：本地/远端共用canonical终态协调与唯一finish门闩；本地失联
仅保留未决记录及只读状态检查。Trace reducer的终态呈现不再旁路写入消息完成。
活动记录与通知改为三态MacRunCompletionStatus，取消不再被Boolean映射成失败；
通知按conversation/run去重。新增本地真实Application提交fixture、composer恢复
门禁和隔离活动/通知测试；运行中验证日志`/tmp/nola-mac-contracts.1p1F02`，尚待结果。

首轮编译仅新fixture嵌套类型遗漏MainActor，补明确隔离后复跑
`/tmp/nola-mac-contracts.CcY8nq`：664项、6个断言失败，全在同一本地迟到终态用例
的EOF/取消/错误三次遍历。该测试只等待recovery记录清除，就读取完整正文和activity；
生产确认持久终态后仍保持isRunning以独占paced drain，记录清除不是显示完成信号。
修订测试等待真实完成边界，不延长timeout、不关闭pacing、不放宽canonical门禁。

复跑`/tmp/nola-mac-contracts.gQiiox`已exit 0：664项中663通过、1项既有opt-in
picker跳过、0失败。新增本地终态4项（含三种中断遍历及只读检查）、composer门禁2项、
活动/通知三态3项及恢复文案1项全部通过。真实Application fixture验证持久完成优先于
迟到取消，未确认中断不写完成活动，旧Run事件不能完成新Run；测试数据库、目录、
书签偏好和通知均隔离。该结果没有覆盖Mac恢复原effect执行、冷启动owner重建、
其他客户端迁移或云端真实模型，相关P4/P5/P6门槛仍保持未完成。

### P4 精确恢复观测与执行身份栅栏（进行中）

新增只读Application `inspect_run_recovery`，要求conversation、expected Run、
assistant三重身份，在同一Store读快照中取得事实并返回typed observation：missing、
identity_mismatch、inconsistent、terminal、active。missing不等于未提交；active只含
持久状态、pending计数、远端来源、会话checkpoint/lease存在性及六态delivery计数，
不包含lease token、owner、工具payload或凭据，也不推断OS存活/授予重试。
Mac状态检查消费该合同，使用原提交时冻结的canonical assistant ID，不从UI UUID
反推存储身份。只读query在Swift receipt-recovery分类与共享command effect均标为查询。

同时发现真实恢复顺序缺陷：CLI原先先处理当前会话的orphan deliveries，再检查用户
指定expected Run。整改在入口先绑定目标，并要求RecoverObservedOrphanedRun显式
携带expected_run_id；Application在实际写事务内复核Run及原delivery身份，拒绝
过期请求误操作新Run。前置读取不能替代这个事务栅栏。真实隔离退出进程fixture
用于验证错误Run不改变ledger；这一批验证尚待全部完成。

Mac子进程当前没有专用expected-Run resume stdin操作。现有`--resume`是交互会话
选择，桌面stdin要求prompt并调用新提交流程，因此不能直接拿来“恢复”。下一步需
独立typed恢复操作和新transport attempt，调用既有expected-Run runtime入口；
保留原有效launch配置并重验已撤销权限，不能读取当前模型中心替换冻结配置。
checkpoint的非密钥binding只能验证、不能反推出完整端点/凭据。冷启动缺少安全
持久快照时必须明确阻塞，不猜配置、不重发用户prompt。

本批验证已完成：CLI permission_recovery_entrypoint_tests 4/4；Application
effect_delivery_application_e2e 6/6、run_recovery_inspection_e2e 3/3、
run_recovery_port_tests 13/13。Mac contracts日志`/tmp/nola-mac-contracts.ms12bs`
exit 0，667项中666通过、1项既有opt-in picker跳过、0失败。新增Mac真实Application
恢复查询及严格DTO解释3项通过，原本地终态4项也验证状态检查读取原Run、无新增请求。
这证明只读观测及错误Run恢复栅栏已接通，不证明Mac已能自动重启原执行；专用桌面
恢复操作、冻结launch配置/权限重验和冷启动恢复仍是明确待办。

### P4 专用桌面恢复协议与原启动所有权（实施中，未验收）

桌面恢复采用独立`operation: resume`，顶层精确session/expected Run/assistant及
configuration；拒绝prompt、new submission、输入items和新context/attachments。
原attachment元数据仅以host_attachments保留原Host配置摘要，不进入新Turn输入。
CLI解析后在任何MCP配置发现之前只读验证身份；恢复分支不走新prompt提交、交互
`--resume`、全局dead-lock清扫或提前启动的通用effect supervisor。

Mac Runtime保留原实际executable/args/environment/serialized配置/toolPlan的内存
launch快照，去掉旧submission ticket与UI回调；恢复共用owned process生命周期，
使用新transport attempt，但Run/message身份不变。工作目录、原授权根与权限模式
在launch前重验，后续UI默认和新增授权不能扩大原快照。当前仅支持内置Core执行
后端的同一Mac进程内恢复；外部executor和应用重启缺少原快照时明确不可恢复。
快照含敏感运行配置，已确认canonical终态或明确删除后需按owner释放，不能一直
留在dictionary，也不能用中间Core Turn结束事件提前删除仍在执行的Run。

Workspace保持原bookmarks选择，读取typed检查结果后显示“恢复原任务”；不调用
send、不改写prompt或重新解析模型中心。取消意图在resume transport登记前不能
丢失；新进程ready后才投递，或launch前重验时拒绝启动。新旧观察generation隔离，
恢复失败仍保留原Run，不写完成Activity。完整退出进程的session lease与delivery
必须在同一事务按精确Run、冻结Host binding和观测owner身份CAS处理；不得先释放
lease再另开事务恢复effects。这条真实崩溃边界正在补齐并验证。

本轮共享事务专项已验证：CLI `permission_recovery_entrypoint_tests` 6/6；
Application `effect_delivery_application_e2e` 6/6、
`run_recovery_inspection_e2e` 3/3、`run_recovery_port_tests` 13/13。
恢复请求的完整冻结Host binding和原session lease均在写事务中复核，lease CAS释放
与delivery恢复共同提交；错误配置、过期owner不能先清锁再失败。真实HTTP冷恢复
必须使用checkpoint冻结的output admission，不能重新探测当前默认配置。

Mac恢复新增终态竞态栅栏：书签解析await前后、开始新generation前及transportReady
都检查精确conversation/Run/assistant与未完成状态；同generation收到持久终态后
不能再启动旧Run。launch前停止恢复仅表示没有启动transport，原Run取消仍待确认，
不能伪造cancelled。终态启动快照清理独立于通知去重，最多三次重读精确持久证据并
由Runtime再验证后释放；不捕获Workspace，不启动任何恢复执行。客户端新增用例和
统一Xcode验收仍待完成，不能用共享层结果替代UI/进程生命周期验收。

未闭环边界：resume launch前取消会阻止新transport，旧Run的cancelRequested却仍然
未获持久确认。只读检查不能产生终态，也不能清除意图后偷偷恢复普通执行。需要独立
cancel-only恢复合同：在精确Run和冻结配置栅栏下先提交Core Interrupt，再只执行
取消effects并等待确认。现有resume入口先执行committed effects、后检查cancel，
不适用于该路径，否则可能在用户停止后先执行一次原模型/工具调用。这是后续必须
完成的执行合同，不以新增文案或当前已通过专项测试视为解决。

CLI桌面协议整组最新复跑33/33通过，包括错误Run/assistant在MCP发现之前拒绝，
以及恢复帧不能携带新用户输入。Mac统一contracts已启动，日志
`/tmp/nola-mac-contracts.uGPa02`；截至本记录仍在构建，尚无通过结论。

该批后续验收记录：`uGPa02`因测试把typed Role传作String编译失败；第二次
`/tmp/nola-mac-contracts.v3pLCl`因新retirement测试未解包UUID?编译失败。
修复测试类型，并把受控gate测试的resume调用改为独立Task避免等待自身门，未放宽
生产合同。第三次`/tmp/nola-mac-contracts.YA3zeF` exit 0：681项中680通过、
1项既有opt-in跳过、0失败。新恢复帧、权限重验、同Run恢复及终态竞态、快照清理
用例均纳入该批。Store `orphaned_run_recovery_e2e` 6/6通过，新增真实SQLite
验证session lease DELETE后的错误回滚，全部五字段和相关十张表保持原样。

上述Mac结果不包含随后新增的Coordinator取消投递重试方法及取消专用合同。
后续实现按两段分工：Application持久取消intent/receipt，受限取消delivery模块
与CLI dispatcher，Mac typed回执与独立重试入口。Host进程退出只证明对应进程
已退出，不证明它启动的外部工具、MCP服务或delegated child已停止；无充分证据的
target必须保留待协调状态，不能利用新进程registry为空生成CancellationConfirmed。

### P4 取消意图与取消投递分离（实施中）

共享新增`request_run_cancellation`，输入只有精确conversation/Run/assistant、
cancellation ID、reason和timestamp，没有模型参数、替换prompt或目录授权。
Application从持久配置恢复并提交Core Interrupt，通过既有received-command事务
保持回执与outbox一起提交。`cancellation_committed`表示意图已提交，可能恰好
由Core立即完成；`already_cancelling`表示仍待确认；`already_terminal`引用已有
终态。返回terminal hint不能替代客户端精确canonical terminal查询。

第一段真实Application测试3/3已通过：未知活跃owner只进入Cancelling、不释放原
delivery lease；相同回执重放不新增事件；错误Run/assistant不写checkpoint；回执
持久化故障回滚Core/outbox。Mac已接typed DTO/port与恢复场景stop路径，在等待
新transportReady之前提交意图，保留原cancellation ID和timestamp以重试不确定回执。
DTO和新的prelaunch-stop集成测试尚待统一编译执行。Coordinator取消投递失败可以
只释放投递claim、不撤回用户意图，终态后拒绝再次claim；新增测试亦待执行。

这仍不是完整cancel-only执行验收：受限delivery facts/claim与CLI dispatcher正在
实现；Mac专用子进程协议及重试按钮尚未接通。旧CLI热Core与另一个Application
实例写入的取消之间依赖checkpoint revision栅栏，需要两Runtime真实回归证明。

后续真实两Runtime测试暴露冷恢复缺口：Interrupt已撤销原target，旧Host迟到响应
仍试图把已ack target重新绑定为pending。整改只在完整原delivery/config身份通过、
持久target已ack、Core确为Cancelling且pending Cancel精确指向该target时，将非终态
迟到响应作为applied=false接收；不追加事件、不生成普通模型continuation、不把它
当取消确认。最终`run_cancellation_e2e` 5/5通过，包括连续两次迟到响应及已完成
任务优先于迟到取消。仅证明共享合同，不替代CLI/Mac进程闭环验收。

Mac新增独立取消control process：只发五字段cancel帧，使用明确DB位置，不进入
原工作目录、不继承原模型凭据或工具授权；并发排空输出、限制保留错误大小、拥有
子进程cleanup及30秒deadline。Runtime活连接必须匹配原Run/assistant，冷连接走
专用CLI入口。恢复卡的“重试取消投递”保留原请求身份，不依赖新目录授权；child
exit仅表示transport结束，界面仍查询canonical终态。此批新增Swift/CLI接口尚待
统一构建及真实隔离进程验收，尚不能据parse通过宣布完整取消链路完成。

本批进一步验证：CLI受限driver及真实取消7/7通过；Application取消6/6、effect
delivery回归6/6通过。新增Application证据表明真实CancellationConfirmed会直接
完成Core与canonical取消，零hook/model/tool successors，不需要修改正确的Core
生命周期或为取消开放用户hook执行。Mac contracts
`/tmp/nola-mac-contracts.6zSwet`已通过新control child三项及本地终态十一项，
包括第二次书签验证期间持久取消、投递失败后撤销目录再重试原intent；整组仍在运行，
最终状态待记录。

另发现取消后原dead session lease可能残留、阻止下一任务准入；正在补真实fixture
及取消claim事务内的精确五字段CAS释放/失败回滚，不允许终态后随意删除会话当前锁。
Mac→实际bundled CLI→同隔离DB的wire验收另设cancel-e2e层，默认contracts不启动
该生产helper测试，不能把纯child协议fixture或内存driver冒充实际产品边界验收。

`6zSwet`的XCTest最终为692项、691通过、1项既有跳过、0失败，Xcode输出
TEST SUCCEEDED；但外层脚本exit 127（运行中新增cancel-e2e分支改变脚本读取偏移，
将注释尾部`unlock`当命令）。这是并行协调失误，不能将该整条命令记为通过。
脚本冻结后需完整重跑，同时打入随后已复现并修复的dead session lease回收。

取消续批已完成CLI专项12/12：精确dead lease原子回收后可取得新lease并准入下一
Run；received回执故障会回滚原lease、target、cancel claims及checkpoint；直接
ApplicationRuntime/public helper缺write UoW时在任何读写前拒绝。Store不再遍历
整Run历史尝试，只按当前取消和target effect身份取latest；历史规模专项另补中。

Mac增加取消持久失败的安全边界：仅中断当前精确Run/assistant所拥有的transport，
不启动新执行、不删除意图、不把物理退出当终态。typed持久结果区分已提交、已有
终态和不可用，错误在后续CancellationError到达时保留。真实SQLite故障fixture
已补，等待最终Xcode执行。focused cancel-e2e首轮编译因nested Fixture缺显式
MainActor失败（`/tmp/nola-mac-cancel-e2e.hL2dtz` exit65），已修，尚待重跑。

真实focused复跑`/tmp/nola-mac-cancel-e2e.U1pibr` exit0，Mac→bundled
nola-agent-host→Application/Store两项通过：never-dispatched模型Run实际完成取消，
Run/messages不重复、原模型attempt未启动；错误assistant不改变12张任务相关表，
允许独立拒绝回执审计。CLI另新增300条合法历史effects/attempts的规模回归通过，
只读取当前精确target即可取消且历史记录逐一不变。完整Mac contracts已另起
`/tmp/nola-mac-contracts.vhZ9zQ`，最终结果尚待记录。

P5 Domain基础合同现已落`assistant_journal`：版本化binary canonical编码、独立
golden fixtures、identity/epoch、连续cursor及重复/缺口/冲突、UTF-8 chunk预算；
JSON的u64序号采用规范十进制字符串。Domain全量68项通过，其中journal新增11项。
这仅是基础合同，不包含typed operation batch、可见性planner、Store日志与chunk
落库、传输或客户端接入，P5仍未完成。下一步不能把opaque batch digest当作完整
操作验证，也不能用当前整assistant snapshot回填日志来掩盖累计复制。

最终本批Mac contracts `vhZ9zQ` exit0：693项中692通过、1项既有opt-in
跳过、0失败，包含取消持久化失败时中断精确owned transport及原intent重试。
这是本批明确的客户端基线，不是整份重构目标完成。Application+Store全量
回归`/tmp/nola-shared-cancellation.t74LjJ`已exit0：116个测试目标共719项通过，
0失败、0忽略，包含文档编译合同；此基线在P5 Store新模块注册前完成。

P5 typed operations已完成领域层：Declare/Append/Replace/Update/Remove、11类
有界metadata、规范二进制编解码与header/batch摘要绑定、owner/generation及
manifest验证边界。validator只读取受影响Part并返回CAS变化，不复制整个assistant
的heads；一万无关Part的专项用例仍只读取一个受影响head。Domain全量80项通过，
补充header便捷接口后专项12项通过。该结果尚不包含生产Store、Application提交、
传输和客户端接入，不能据此声称P5端到端完成。

### P5 接入前的精确消息归属修正

Application实际允许一个Run产生多个successor assistant消息；journal不能把
`runs.assistant_message_id`（初始消息）误当成唯一合法owner。与此同时，旧
`commit_transition`的Part owner只校验同会话，assistant状态更新仅按message ID，
会允许错误角色/其他Run的消息进入更新路径。现将消息声明、Part owner和状态更新
统一限定到精确Run，状态更新还校验conversation、assistant角色及实际更新行数。
合法successor依赖`messages.run_id`证明归属，不依赖消息ID命名猜测。

`CARGO_INCREMENTAL=0 cargo test -p nola_store --test ledger_e2e`（72407）exit0，
17/17通过，新增拒绝错误owner后事务回滚及合法successor写入两项。此前75006
失败为fixture在活动Run期间导入历史（被既有合同正确拒绝），已改为先导入再Begin；
不是忽略失败测试。该结果尚未覆盖Application全量或journal生产接入。

Mac parser专项构建`/tmp/nola-markdown-tail.Zv9Zzl`exit65：Rust静态库归档因
磁盘空间不足失败，Swift/XCTest未执行。确认无活跃编译后，仅清理两处生成的
Rust incremental缓存，未删源码、数据库或测试证据；缓存可重编生成。生产scanner
独立Swift编译/执行3组合同通过，完整parser/UI suite仍待验证。

P5 Store源端首批已注册：schema27→28、本地epoch、typed Admission/Transition
提交证明、chunks/manifests、Part tombstones、固定水位及snapshot owner引用验证。
56485新增journal11/11通过；78404 Store lib177/177、conversation_resource_e2e
5/5通过，包含真实旧schema迁移。2/8MiB验证目前证明叶子层写入payload/refs随
增量增长，不证明Application或客户端整体线性。snapshot目前长期固定，尚未实现
释放/过期/GC；不能宣称完整生命周期验收。

Application已注册Text planner与事务publication bridge；bridge使用规划时的cursor
而非发布前重读来绕过CAS，准备manifest后比对计划root，再使用Store实际提交证明。
尚未启用生产journal消费者：非Text可见种类、完整提交点覆盖和大单次变化分批仍待
接齐，不以静默忽略其他种类换取Text专项通过。

Application Text planner专项66183/98328两轮8/8通过；publication真实SQLite
集成42485两项通过：超过256KiB的Unicode正文分段准备、同一提交发布、冷重启后
固定snapshot逐chunk重建；错误stamp发布失败后外层事务回滚，重新准入/发布成功。
这仍是接入模块合同测试，非真实模型或全类型生产发布验证。

Mac parser定向重跑`/tmp/nola-markdown-tail.YLQB0Q`（96287）exit0，219项中
218通过、1跳过、0失败，包含新scanner7项、MarkdownParser26项、StreamingRegression
7项及WorkspaceBehavior179项。完整Mac contracts另行验证，不用此专项替代全范围。

后续publication71998三项通过，新增review Pending→Approved后的快照权限合同：
旧snapshot仍无body，不能拿已知新reference读取新正文。Application全lib随后
151/151通过（含Reasoning增量规划3项），这是正文field generation扩展前的基线。

Store snapshot目录分页改为限量目录ID＋每ID固定水位索引查询；93541 journal
15/15通过。2048次实际metadata revision前后，两条查询SQLite VM步数仍为
`(20,19)`，EXPLAIN为SEARCH INDEX；晚创建/tombstone可产生空结果页但cursor
必须继续前进，StorePort已明确此合同。32MiB样本实际正文33,554,432字节、512个
chunk引用，journal每记录有界，末尾chunk读取通过；不是全端线性性能结论。

完整Mac contracts `splDuh`（58363）exit0，xcresult核对726项：725通过、1跳过、
0失败、0运行时警告；其中XCTest700项、SwiftTesting26项。运行/恢复/取消和新增
Markdown scanner均在此回归中，脚本正常退出。此结果不包含尚在Rust侧开发的新
field-generation发布模型的生产Mac接入，后续接线仍需重新验证。

字段更新合同新增ReplaceContent：字段正文generation使用新head.revision分配，
删除后重加不能复活旧root；整体Part替换可保留完全相同的既有refs。Domain专项
16/16通过。Store新增真实2MiB工具输入＋32次输出替换回归，输入manifest始终32项，
Part generation不被输出更新推进；此项包含于41094 journal19/19通过。

snapshot release独立生命周期已落schema29：完整owner/sequence/digest校验，
Released/AlreadyReleased幂等，释放后拒绝读取且不删其他snapshot/正文。真实28→29
以及旧27/26/25链迁移已测；资源迁移5/5通过。release不等于内容GC，后者仍未实现。

P1本地MLX边界复核确认请求级生成参数被HTTP解码遗漏，导致启动默认覆盖会话值，
正在修复。另将Mac launch内endpoint解析统一一次且严格传播不可用错误，不再try?
后回退远端参数；恢复仍使用原launch snapshot。新endpoint三项测试已加入、语法
检查通过，XCTest尚待执行；发送准备前完整runtime lease仍未完成，不混同此修复。

P1 MLX准备期实例快照续批：Workspace在模型ready之后、异步workspace解析之前
捕获Endpoint，将其作为不可变RunRequest字段；launch将当前实例与捕获的model、
port、token、context精确比较，不相符则拒绝，不选择新实例。新提交的arguments
和stdin共用同一解析结果，resume仍复用原launch snapshot；外部executor不要求
MLX endpoint。新增restart/mismatch合同，语法检查通过，XCTest尚待确认。

MLX请求级参数resolver已消费请求覆盖而非仅启动默认，独立生产源测试通过。
统一Mac定向构建`DWDjia`（68016）exit65，ChatRequest Decodable编译失败；已修
Swift.Decoder类型限定，须以重新编译结果验收，不能以独立resolver通过代替实际target。

MLX实际target后续54368已SwiftCompile/EmitModule/Ld成功，但Mac测试被同时编辑
的Rust range接口未注册完整阻住（4个编译错误），该次exit65不能记作Mac通过。
现按跨层构建前生产代码freeze协调重跑。另修复本地MLX在温度/top_p恰好等于UI
默认值时省略实际请求参数的问题：已解析本地请求始终明确发送sampling值，云路由
仍保持原capability/override语义；新增payload回归，尚待Mac执行。

P5 Reasoning/Tool/Source/Citation/Artifact planner扩展已通过Application专项
65426的19/19测试（planner16＋真实publication3）。Domain 90636的32/32通过，
含typed大字段引用、结构化JSON完整数值、单端引用范围与独立JS ReplaceContent
golden。工具output变更保留input refs，structured output优先于preview；仍未接齐
Step/Compaction/Task/Skill/Attempt及所有生产提交点，不能切换消费者。

Store manifest byte-range 61717四项通过：schema29→30索引迁移、精确snapshot/ref
授权、UTF-8边界、跨chunk和释放后拒绝。2/32MiB前缀后读取16字节的实际SQLite
VM steps/读取chunk字节均为(44,65536)。该底层按固定root定位，不从chunk0扫描；
旧canonical字段的全文SHA/SQL拼接、原文读取和同步发布仍待一起迁移。

P1 Mac/MLX联合验收最终30138 `XUuih5` exit0：52项全部通过（恢复授权2、恢复帧3、
endpoint4、本地route2、终态12、reasoning配置29），包含本地强制发送已解析sampling
值。实际MLX main/resolver在前次54368已编译链接，此次完整依赖与测试流程正常。
新增`./scripts/test_mlx_generation.sh`编译同一生产resolver、无需模型/网络/凭据，
42945执行通过，覆盖默认继承、显式零、范围、别名冲突及unsupported请求；临时
二进制运行后自动清理。这不证明真实模型端到端效果或P1其他客户端全部迁移。

### P2 本地模型响应编码所有权

复核[OpenAI function calling streaming合同](https://developers.openai.com/api/docs/guides/function-calling)
与[Chat Completions响应合同](https://developers.openai.com/api/reference/resources/chat)：
tool delta的index标识调用，finish_reason区分stop/length/tool_calls。旧MLX服务把所有
tool index写为0、流结束固定stop，且SSE开始后catch还发送第二个HTTP响应。

现抽取MLXChatResponse，独立管理稳定completion身份、工具序号和终态编码；消费
实际SDK GenerateCompletionInfo中的stopReason与token counts，取消/缺失完成信息
不能伪装成功，有usage请求时输出独立usage chunk。main catch按HTTP是否开始选择
同一SSE错误或首次JSON错误，错误后不发送成功DONE。移除工具参数序列化失败时
默默替换为空对象的行为。

`./scripts/test_mlx_response.sh`（58510）编译生产codec并执行通过，覆盖两工具索引、
稳定envelope、usage、长度终态、取消/缺失info、重复结束拒绝及nonstream一致性。
main语法检查通过；此批实际MLX target重新编译和真实模型stream仍待验收，不复用
上一批52项测试声称新响应编码已通过集成测试。

进一步将 HTTP/SSE 写入归属抽为实际接入 main 的 `MLXHTTPResponseWriter`：
idle → streaming → finished/failed；JSON 也必须在首次写入前占有响应。
传输失败可能已有字节到达对端，因此不能重发 header、追加错误响应或重试 DONE。
只有尚未开始响应的推理错误可产生 HTTP 400；已开始但传输仍正常的流可发送一次
SSE error 后关闭。连接关闭仍由 main 的 defer 负责，writer 不拥有模型任务生命周期。

5513 `scripts/test_mlx_response.sh` exit0：同一生产 codec 和 writer 均通过；新增
注入传输测试覆盖正常结束、重复结束/事件拒绝、推理中断、header/event/DONE 写入
失败、非流式部分写入失败不发送第二个响应、响应前错误。此为真实生产 writer 的
确定性故障注入，不是 Network socket 或真实模型端到端验收。

补充验证：54864在测试入口显式分离非actor测试体后再次通过；生产writer与测试的
Swift 6 strict typecheck通过。实际MLX target构建80026因依赖yyjson的空index-store
路径失败，尚未验证新main；禁用编译索引后的59302构建仍在执行，不能记作成功。

后续59302已exit65：单target依赖扫描无法解析NIOCore所需DequeModule/Atomics，
仍未验证main。改用实际NolaMac scheme的62725构建完整依赖图，结果待定；不重复
同一失败命令、不把依赖构建失败解释为新响应代码编译成功。

62725最终exit0、BUILD SUCCEEDED：实际NolaMac scheme完整依赖编译通过，日志明确
编译main.swift、MLXChatResponse.swift与MLXHTTPResponseWriter.swift。这补齐本批
真实SDK编译证据，但未运行真实模型或socket端到端验收。

iOS本地Run独立provider已通过11067真实模拟器构建和3项测试（0失败/0跳过，
`/tmp/nola-ios-local-run-20260928.xcresult`）：会话参数冻结、零值与模型身份验证。
权限等待后的恢复仍读取共享provider及当前配置，属于后续必须修复的独立断层；
不能因Core原config digest正确而推定外层模型调用也已冻结。

共享恢复设计与数据边界详见 `IOS_RUN_MODEL_RECOVERY.md`。恢复应复用并扩展已随
AgentTurnIdentity持久化的HostInvocationBinding，而不是新建第二套iOS模型快照。
现binding中reasoning/options仅参与CLI policy摘要，不能从摘要重建原参数；还需
可重建字段与精确Run只读接口，完整恢复链目前未实施。

先补现合同安全前置条件：HostInvocationBinding::validate现在真正校验非空
endpoint为规范化HTTP(S)，无userinfo/query/fragment；不能信任仅CLI构造器做过
脱敏。空endpoint保留现有本地构造语义，但不得携带query digest。43635专项8/8
通过，覆盖primary/fallback、敏感URL、规范化、空本地地址和query摘要；错误信息
不回显地址内容。这不证明endpoint path任意内容均不敏感，也不代表恢复已完成。

P1恢复只读边界已接入Application→Store：`LoadExactRunInvocation`三identity查询，
同snapshot拒绝错误owner/remote，并在Application验证原config摘要和typed binding。
只返回可公开绑定，不导出原始Core配置。28151真实DB 2/2通过；Swift恢复与v3可重建
字段仍待接通，现v2比较绑定不等价于完整可恢复模型配置。详细边界见iOS恢复设计。

精确读取Store最终18746全195/195通过，新3项包含冷重启、错误owner/remote拒绝、
真实WAL pinned snapshot一致性。Application42117的3/3通过，新增提交安全前置：
`invocation_config_snapshot`在序列化/创建Run之前验证已提供的Host binding，普通与
maintenance admission共用此入口；敏感query地址被拒绝时conversations/messages/
runs/effect_outbox均无写入。缺binding旧路径尚未强制升级为可恢复合同，恢复读取
仍明确拒绝；不应把已有新发送路径支持误报成冷恢复可用。

HostInvocationBinding v3生成字段切片现已实施：reasoning_selection（required nullable）、
sampling_override与ValidatedGenerationOptions显式持久化；后者复用Domain options
校验与规范化，不为f64手写Eq。CLI真实构造同步移出policy摘要重复字段，旧v2不能
冒充v3；仍缺route/protocol/credential可重建引用，不宣称完整冷恢复。专项Domain6、
HostProtocol26、Application9及CLI真实binding1均通过；root31740最终Application
全lib161/161通过。

Swift侧已接只读envelope与claim前显式准备gate：默认缺prepareRecovery则拒绝，
准备后再比对精确绑定；新增4项测试目前仅parse，实际模拟器运行待排。生产尚未
提供完整provider准备器，因此冷恢复暂时拒绝，不能使用当前客户端执行旧Run。

路由审查发现真实协议分叉：streaming先检测Anthropic，而token字段/continuation先
判断Responses；相同配置可能走不同协议。统一resolver切片正在实施，并要求实际
dispatch、token字段及continuation共同消费，不只改变日志标签。
[OpenAI官方streaming合同](https://developers.openai.com/api/docs/guides/streaming-responses)
确认Responses有独立typed生命周期事件；协议分派必须与对应解码器一致。此处是
基于本地分派代码得出的整改设计，不代表官方资料为第三方endpoint兼容性背书。

该协议统一切片已完成：66111七项合同/真实HTTP测试通过；10585 LLM全108/108
通过。三处实际消费统一resolver，显式协议规范化、endpoint精确host识别；未将
fallback继承主能力的遗留问题误标为已修。另发现URL query路径拼接根因，后续修复中。

本地MLX请求流式选择抽为MLXStreamRequest，修复缺省stream被错误当作true：默认
普通JSON，显式true才SSE，usage只在流式且请求时开启。79996独立脚本的响应codec、
HTTP writer与请求解析三组均通过，main parse通过。本条修改晚于62725实际build，
因此新的请求解析target集成编译仍待验证，不能沿用先前build成功作为证明。

66141 iOS真实模拟器专项已exit0，11/11（新4＋原7），验证精确读取/准备失败零claim
及既有fencing；仍没有完整provider重建，生产默认明确拒绝冷恢复。大整数seed在
AgentCoreJSONValue Double通道保真问题已记录，不能用该通道盲目重建i64生成参数。

URL规范化切片41021的LLM全112/112通过：统一修改URL path并保留query（含重复键
与编码值），三协议真实HTTP覆盖base/full/trailing；拒绝userinfo/fragment，诊断和
网络错误不回显query。此验收先于下面的Host字段变更。

HostInvocationBinding v4、HostOutputAdmission v2加入required ModelProtocol字段。
CLI准入与binding构造复用实际dispatcher的resolved_model_protocol，不再从标签另猜。
25550 HostProtocol全27/27通过，覆盖缺字段拒绝、admission协议一致、协议改变破坏
exact身份，以及旧版本拒绝。CLI联合47934准入8/8通过；route/credential引用等
仍不齐，不能把新增protocol字段标为完整冷恢复实现。

v4最终40345 Application全lib161＋effect delivery6＋exact读取3全部通过。
后续v5（OutputAdmission v3）增加最终ModelOutputTokenField：它不是偏好Auto，而是
实际JSON字段。准入和请求构造共用窄resolver，字段必须与protocol相容；所有候选
route均包含它，变更会破坏exact身份。retry/parallel请求策略也纳入同批显式绑定；
缺少retry准入值时禁止绑定构造器回退读取环境。CLI实际HTTP冷恢复重试回归正在跑，
尚不记为通过。本批仍不包含完整凭据引用/provider恢复，不能启用iOS默认准备器。

后续实际HTTP34279及完整permission恢复61443通过：env retry=1准入，改为0并释放
Host，同Run恢复面对503仍按原retry=1成功；未重复执行原工具。该验收还暴露并修复
driver把parallel None覆盖成capability布尔的生产问题。初始None现在保持省略，显式
偏好只允许被step约束收紧，已观察oversize后的显式false纠错仍保留。最终Host28、
output准入9、偏好6组合矩阵1、permission恢复6均通过。

47715 exact读取3/3通过，增加context_budget：直接复用Domain ContextBudgetFacts
及原Core context_window投影的7字段，不从当前模型上限/输出偏好反推。测试验证
nondefault配置冷重开后原值保留，并刻意区分reserved_output_tokens和generation
max_output_tokens。Swift opaque envelope正在同步新增required键。

下一tagged route片已批准实施：HTTP与native分支明确分离，本地MLX不能用假URL或
ChatCompletions协议填充绑定。native只冻结adapter合同与逻辑模型选择，不承诺权重
字节快照。新发送PreparedRunModel将统一一次解析的client/capabilities/parameters；
这两片仍在实施，完整iOS provider恢复和凭据引用尚未接通。

本轮并行落地：Invocation v6 / OutputAdmission v4 已明确 HTTP/native tagged route，
Host 校验与 CLI fixtures 同步。iOS 实际模拟器专项 17/17 通过（prepared 2、exact 5、
local 3、canonical 7）；该验证不代表冷恢复 provider 已接通。

输出准入进一步集中为 `HostOutputAdmission::admit(routes, preference)`，CLI 使用该
构造器；Application 增加只读 `admit_host_output` 查询供 native bridge 调用。客户端
提交真实 route/capability 与输出偏好，不自行填写版本或 resolved.source。此查询
不读模型目录、不持久化、不授予工具权限；context/input 预算不是 output capability。
暂不新增重复的 generation draft，也不改变 configured fallback 的身份比较规则。
完整 invocation 仍需在权限/tool policy 齐备后组装，并经原 begin 事务验证、持久化。
串行验证 31315 已确认 Host 32/32、Application 准入查询 1/1（包含无数据库变更检查）；
CLI 准入专项 10/10 通过，31315 最终 exit 0。Swift 调用新查询和完整凭据重建仍是
后续接入，不算本轮完成。

### P1 恢复边界补充：执行意图、实时证据与凭据

当前源码审查确认，iOS `prepareBoundPlan` 的 manifest ID 每次随机生成，60 秒过期；
`policyDigest` 对包含时间、系统授权、临时 grants 的整个 input 求摘要。它是本次计划
证据身份，不是可跨恢复复用的执行配置身份。CLI binding 中完整 device advertisements
同样含 egress receipt/grant；不能原样复制到 iOS 作为稳定执行策略合同。

后续接入必须分开：

- 持久执行意图：原 Run permission profile、执行模式、Host/client/session 身份、
  显式工具/能力 allow/deny、handler/schema 版本、资源范围及 backend。集合顺序规范化。
- 实时授权证据：manifest UUID/签发和到期时间、前后台和系统授权状态、临时 grant、
  result egress receipt 与当前撤权状态。恢复时重新验证，不因旧摘要相等而跳过。

不能以已被实时授权过滤的 enabled_tools 反推原执行意图。恢复从原 Run 获取意图，
新计划不得扩大原范围；每次执行继续检查 plan 有效性。旧 Core allow rule 不是平台
权限的永久许可。此分离尚未完成生产接入，不能据此启用 iOS 默认恢复 gate。

凭据引用复用已有存储而非另存秘密：CLI vault 的 endpoint ID/revision，Mac 固定
Keychain service 下的版本化 account，iOS 固定 service 下的 profile ID。引用类型
必须封闭，不能接受任意 service/环境变量/路径；digest 仅用于读取后核对，不能重建
凭据。Mac 当前 resume 复用含 key 的内存快照，需在 dispatch 前重查 endpoint/account
及撤权。iOS account 会原地覆盖，现有设计无法区分删除后同 key 重建；需要独立版本
才可承诺该撤权语义。跨 Keychain/DB 不存在原子事务，保证边界是发送前最后复验，
不能宣称能撤回已发送请求。临时 key 无 durable reference 时不得伪装成可冷恢复。

本轮参考代码核对：`参考资料/codex-main/codex-rs/core/src/client.rs` 的
`current_client_setup` 从 provider 解析当前 auth/provider，并供 prewarm 与连接共用；
`参考资料/opencode-dev/packages/opencode/src/auth/index.ts` 将 get/all/set/remove
集中在 Auth service。这里借鉴的是单一凭据解析入口和生命周期所有权，不据此推断
参考项目承诺了 Nola 所需的版本化撤权语义。Nola 的恢复仍必须绑定原 profile/account，
不能把“重新读凭据”实现成“静默换到当前选中的模型”。

本轮参数保真修复已进入 `OpenAIProvider` 实际请求构造：非法/非有限 penalties 和
超限 stop 拒绝，合法值不 clamp、不截断、不 trim；Responses 原不支持项仍拒绝。
新增请求 JSON 测试 4 项。通用 Domain 合同为 stop 最多 16 项/每项 2048 UTF-8 bytes，
不是所有 provider 的支持承诺；[官方 Chat 文档](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
规定 stop 最多 4 项，provider 专属 capability 上限仍需后续显式建模，禁止根据模型
名字猜测或静默缩短列表。iOS 租约整数读取也改为 exact conversion：原
`Double(Int64.max)` 边界比较会接受 2^63 再 trap，现明确拒绝并支持 JSON 原生整数。
新 canonical 回归覆盖该边界与 NaN/Infinity/unsigned 溢出。82643 模拟器专项 exit65：
40 项中 34 通过、6 崩溃、0 跳过；新 canonical 边界测试通过。崩溃包括新参数保真
4 项与旧 OpenAIProvider 的 2 项 fail-closed 测试，xcresult 指向 XCTest 错误调用路径，
后续 ips 栈显示 OpenAIProvider 析构进入 `swift_task_deinitOnExecutorMainActorBackDeploy`
并触发 malloc 非法释放；成功构造请求的临时实例同样触发，不是只在抛错时发生。
修复显式 `nonisolated deinit`（仅持有不可变配置与线程安全服务，无 actor-bound 清理），
保留临时实例释放测试，不以延长对象生命周期掩盖错误。修复尚待同批模拟器复验，
不能归为测试环境问题或声称参数修复已通过。首轮结果位于
`/tmp/nola-ios-generation-fidelity-20260928.xcresult`。MLX 三组独立合同 77378 已 exit 0；完整
Mac 构建等待当前 iOS 编译窗口及凭据修复源码冻结。

后续同40项模拟器复跑82995 exit0，xcresult确认40通过、0失败、0跳过，原6项析构
崩溃全部消失（详见 `IOS_PROVIDER_DEINITIALIZATION_REGRESSION.md`），没有删除或
延长生命周期绕过测试。Mac19343已开始实际scheme构建与凭据5＋恢复5项专项，尚未
记录为通过；此构建也覆盖新 `MLXStreamRequest` 的真实target编译。

Mac 首轮19343生产target编译完成（包含 `MLXStreamRequest` 和 runtime main），
新增测试错误使用模块名 NolaMac 导致 test 编译失败；已按现有测试改为 Nola。
后续50724 actual scheme test exit0，凭据5＋恢复授权2＋恢复frame3共10项通过。
凭据检查使用真实隔离配置库和注入固定account读取器、同生产dispatch gate的launch
spy，未访问真实Keychain/模型或启动CLI进程。所有configured fallback暂保守复验，
与已冻结配置范围一致；后续需接共享可执行候选集合，不能在Swift复制协议eligibility。
该片保证恢复进程启动前复验，不等于长期子进程每个模型HTTP请求均已接撤权检查。

复核补充：capture 不仅比 endpoint/model/key，还必须比实际 request 的 wire protocol。
否则准备期间 profile 从 Chat 改为 Responses，会把旧请求与新 profile 错误组合。
已增加该拒绝条件及准备阶段协议变化回归，同时扩展恢复时 endpoint/model/protocol
变化的拒绝矩阵；74292 exit0，Mac 11 项全部通过、0跳过。fallback 的实际协议按现有路由合同继承
primary，因此也核对这一事实，不从 endpoint/model 名字猜测。

下一实际纵向修复已启动：iOS Run 只创建一次60秒 plan，coordinator、权限卡及executor
持续使用旧receipt，长推理/长时间权限确认后无法继续。Run级plan owner应冻结原始
candidate schemas/handler、identity及allow/deny意图，刷新时仅重新观察平台权限与
manifest；原scope/egress授权不得续期。工具执行、权限卡提交和异步资源选择返回均
使用刷新后同一challenge receipt，原requestID/callID和durable effect不变。该方案
正在实现，不代表完整持久execution policy摘要或冷恢复已接通。

CLI 对应检查发现更严格的前置条件：原 device bridge 只在目录解析时验证 egress
receipt/grant；实际 `execute_tool_inner` 直接交 resolver，native bridge 创建300秒
challenge，收结果只校验challenge identity/nonce/expiry。仅从binding删除临时证据
会让已过期目录继续执行，因此必须先把共享证据校验接到派发和结果消费两侧。
challenge截止时间不得超过原receipt/grant，毫秒转换为秒不能向上延长授权；消费
必须再次验证原始毫秒期限。完成这些检查后，稳定策略才可只保留工具schema/capability
和exact host/client/session/tool/route意图。当前TLS保存的是单次请求快照，没有可
查询的会话内egress撤销registry；本片不能声称解决了任意即时撤销，仍需后续owned
authority。实现和验收正在进行，不能把计划当成已落地的保证。

计划刷新并发约束：actor 本身不能防止 `await observe()` 重入。旧观察晚到不得覆盖
新receipt；但简单“generation失配就递归再观察”也可能让两个调用相互失效而活锁。
采用single-flight观察/刷新任务或显式串行队列，并保留executor最终live检查。
UI权限链还必须绑定原coordinator/Run及picker challenge token，所有await返回后做
身份核对；不能提交时重新读取当前coordinator，或让旧picker回调清除新Run的权限卡。
这些约束由本轮源代码审查发现，正在补进实现和可控时钟/continuation回归。

本片验收不能只跑plan session单测，必须覆盖实际消费者：

- 挂起授权观察后取消Run，再返回观察：executor调用次数为0，pending permission不复活。
- 旧权限窗口尚未返回时切换Run，旧picker回调不得注册grant、批准新challenge或清新UI。
- 多个currentPlan调用共享单次观察并有界完成；随后撤权观察必须生效。
- scoped/egress原期限不变，精确到期即失效，manifest刷新不能成为新同意。
- CLI派发前过期不发送challenge；已发送后授权过期不接受/发布结果；原身份不符拒绝。
- 有效且成对更新的短期证据可用于原执行意图，schema/capability/owner变更仍破坏绑定。

恢复身份、授权实时性、消费者副作用边界必须分别证明，不能用其中一组绿色测试替代
另外两组。真实模型/设备端到端验收仍在总体计划中，并未被这些合同测试取代。

共享egress谓词已提取，9954 Core tool_policy 14/14通过；CLI三处live边界及owned
pending proof已接入，14422 CLI device_egress专项2/2通过（原句柄exit0），完整CLI
回归尚未终验。复核要求补nonce-scoped清理guard：发送
失败、等待锁失败与其他早退都清除自己的challenge/waiter，不留授权proof悬挂。
另外binding的期望owner不能从待校验receipt自身构造；将认证连接的RuntimeIdentity
与resolver放进同一作用域，并测试嵌套退出恢复。该所有权修复仍在进行。

最终设备授权片验证：8742稳定binding恢复1/1，12390 CLI device_专项11/11，13762
Core tool_policy 14/14，均exit0。native dispatch、HTTP接受、worker发布结果均复用
同一谓词；pending保存认证连接的owned proof，deadline按原毫秒授权限制，发送前
再次检查；nonce-scoped RAII清理覆盖断连和早退。binding所需identity来自认证resolver
作用域而非receipt自身；resolver/tools的TLS guard显式 !Send/!Sync，编译期断言及
嵌套恢复回归均包含。本片没有新增TLS外的全局撤销registry，不能声称即时撤权已闭合。

iOS对应生产片已完成语法检查并冻结：single-flight plan owner、immutable原选择、
coordinator生命周期fence、同步可撤销dispatch permit、原coordinator/request绑定的
UI权限回调。新增session4＋permit2测试，实际coordinator挂起取消测试正在补齐；
尚待统一Xcode实际运行。permit只阻止撤销后的新准入，不追溯撤回已准入的系统操作。

### P5 下一纵向切片：canonical Text 流式字段

CanonicalFieldReader 已接入现有 Inline/Sealed 读取，Store 最终19101的192项通过。
这不表示流式字段已分块：Application 与 Store 的 Append 仍计算完整前缀 SHA，
Store 仍 SQL 拼接累计正文，Close 才完成最终 JSON 封存。

下一切片必须同时改变以下生产边界，不能仅增加可测试的独立存储 API：

1. canonical 字段 binding 明确 owner、原始字段、generation/revision、root、长度；
   不得直接把公开 journal ToolInput/Output 引用当原始 content_json。
2. TextDelta/TextCommittedDelta 与 Close 一起切换到字段 head CAS＋新增 chunks；
   Close 可进行一次全文封存，Append 不得扫描旧前缀。
3. 已有 Chunked 字段的普通 Upsert 必须区分 metadata-only 与显式 Replace，不能
   悄悄恢复 content_text/content_json 全文双写。
4. 正文 hydration、context/tail、bounded preview、range、search、export、branch、
   semantic source 和 derivation 均识别真实绑定，不能将空 inline slot 当空正文。
5. 同步 source→replica 只发送新增 chunk/manifest 和 head CAS；缺依赖不推进 revision，
   不得复用每次遍历全文的 sealed enqueue。

验收必须走 Application→Store→replica→冷重启，再测正文消费者；对2/8/32MiB追加
记录每次读写量，验证错 owner、旧 root、重放、缺 chunk 和事务回滚。同时单独记录
Core checkpoint 全量序列化成本，禁止把字段存储优化声称为端到端线性复杂度。

### 并行复核：三档准入旁路与记忆证据

Mac Composer 已只呈现三档，主提交路径将 tier 交共享 Application resolver。
本轮发现的遗漏不是新增 UI 控件，而是两个调用者仍走旧解释路径：
会话参数编辑器要求未选中的 budget 也小于本次输出上限；模型测试页未传 tier。
编辑器已改为预览和保存均调用共享 resolver 校验所选 tier，新增隔离数据库回归；
模型测试页修复和统一 Mac 构建待验证，不能以 Swift parse 通过作为验收。

默认 Mac 支持目录的最近 canonical 会话时间为 2026-09-25 20:40（北京时间），
不是数据库 WAL 最近修改的时间。该会话存在记忆使用 ledger，但无工具执行 trace；
这能证明驻留记忆使用，不能证明模型主动调用或用户关闭失效。当前生产链从会话
override/全局默认冻结 memoryEnabled，经 CLI policy 到共享 selector；关闭时
selector 返回空 selected，独立召回环境变量不能绕过该策略。历史缺少完整调用
快照，不能用当前代码反推当时配置。当前会话级 memory override 缺少撤销入口仍是
交互缺口，应置于 Inspector 的继承/启用/禁用设置，而非重新添加 Composer 控件。

iOS 生命周期首轮实际编译失败仅定位到新增测试缺少 try，已修正。第二轮
`/tmp/nola-ios-tool-plan-lifecycle-fixed-20260928.xcresult` 实际编译通过但测试失败：
fixture 缺少 result-egress 批准使文件工具未启用；取消后的 API 返回 typed
permissionNotPending，而非 nil。第二项观察失败后的测试等待还会永挂，故明确中断
该次运行，不能记为通过。已修 fixture 授权前提（不授予文件 scope）、错误断言及
提前释放/超时路径，第三轮 8 项专项报告为
`/tmp/nola-ios-tool-plan-lifecycle-v3-20260928.xcresult`。第三轮实际 7 项通过、1 项失败，
不再永挂；它暴露了生产合同问题，而非又一个工具 fixture 问题：

- `HostRequestEnvelope.attempt_id` 是可缺省的模型尝试身份；`delivery.attempt_id`
  是每个已认领 effect 的物理投递身份。合法权限 effect 没有模型尝试 ID。
- iOS claim 校验曾错误要求顶层 attempt_id；恢复校验又要求它等于 delivery ID。
  既有 stub 填了两个相同 ID，掩盖真实 Application 到权限请求的失败。
- 生产 claim/recovery 已统一校验 delivery 的 ID、worker generation、owner、token
  和精确整型到期时间，不再要求与可选模型尝试混同。模型执行自己的 attempt 校验
  保留。待覆盖无模型 attempt、不同 ID、外来/过期/缺失租约及实际权限取消路径。

Mac Inspector 已增加会话记忆的默认继承/启用/禁用入口，不往 Composer 增加选项，
不把读取默认值变成持久化 override。新增持久化/会话隔离与已构造请求冻结测试；
仍待统一 Mac XCTest，后者不能替代完整发送 E2E。

Mac 最终定向验证：`/tmp/nola-mac-tier-memory-20260928.xcresult`，NolaMac 实际构建
与 29 项测试全部通过（0 skipped，原执行句柄 45933 exit0）。覆盖会话配置原有
22 项及新增三档准入 2、模型测试请求解析 3、记忆 override 2 项；未执行 Inspector
可视化验收或真实模型请求，不能以此宣称完整 UI/E2E 已完成。

跨端同根因审查发现 Android 的 claim、permission recovery、exactly-once admission
及 heartbeat 也错误依赖模型 attempt 与物理 delivery attempt 相同；旧测试把两值
伪造相同，甚至将不同值当失败条件。Android 修复中。Mac/Web 执行路径下沉 CLI，
当前核查 CLI 保持顶层 attempt 与 delivery 分开回传，未发现相同条件。iOS 修复版
统一报告 `/tmp/nola-ios-delivery-identity-20260928.xcresult` 正在执行。

该 iOS 统一轮最终 70/71 通过、1 失败、0 skipped，exit65。唯一失败为新增测试要求
真实 permission claim，但 fixture 使用 recommended 自动放行读取，实际暂停于工具
执行前的观察；生产身份校验的 11 项合同测试全部通过。fixture 已明确 `.safe`，
保留真实权限 claim、无模型 attempt 与取消后不复活的断言，未弱化验收；同一 71 项
重跑报告 `/tmp/nola-ios-delivery-safe-20260928.xcresult`，待最终结果。Android 同类
修复和回归已编写，另补相对认领时刻的到期检查；尚未运行 Gradle。

本片最终 iOS 统一验证：上述 safe 报告 71/71 全通过，0 failed/skipped，原句柄
66777 exit0；包含真实 Application 权限 claim 后暂停观察并取消，权限请求确实无
模型 attempt。Android 两类专项 9/9 通过，`testDebugUnitTest --tests ...` 原句柄
50165 exit0，报告在 `android/app/build/test-results/testDebugUnitTest/`；随后启动
完整 debug unit suite（非设备 instrumentation），结果另记。上述验证仍不证明
冷启动 provider 重建、OS picker UI 自动化或真实模型 E2E 已完成。

Android 完整 debug 单元测试随后完成：82 个测试类、303 项（301 通过、2 skipped、
0 failures/errors），`JAVA_HOME=.../temurin-21.jdk/Contents/Home ./gradlew
testDebugUnitTest --no-daemon`，原句柄 38527 exit0。统计来自当次 JUnit XML，不能
把 2 skipped 算作通过，也不包含 release 或真实设备 instrumentation。

### P1 后续生产闭环：精确模型重建

本轮只读接线复核确认 `IOSPreparedRunModel.prepare` 是下一修改入口，而不是新建
独立快照工具：它必须在一次读取中保留已解析 profile ID/revision 和真实 generation
事实，先经已有 `AdmitHostOutput` 再构造 provider。`makeRuntimeConfig` 必须将完整
binding 与 canonical begin 原子提交，不能依赖 prompt identity 非空才保存。

恢复缺口仍不能以 JSON 填字段解决。HTTP binding 只有 credential/query digest，
哈希不是可取回的 secret 引用；iOS Keychain 当前 modelID 槽位可变，读当前同名模型
会改变原 Run。应由凭据存储所有者提供可信引用和版本，先准备新版本，再原子发布
profile 引用；引用校验/缺失/轮换失败必须明确拒绝，不能改用当前 key。含敏感 query
的 endpoint 也需安全重建来源。Native 的逻辑 modelID 不等于权重内容快照。

实际调用链为 ChatManager → prepared model → Coordinator → canonical begin；恢复
复用现有 exact invocation 查询及 prepareRecovery 后二次一致性检查。要同时证明
更改当前模型不改变旧 Run、凭据轮换不串用、无 prompt identity 仍持久化、HTTP/native
分别重建，以及撤销权限不新建 Run。在这些条件未实现之前，保留当前拒绝不完整
冷恢复的 gate，不用当前 `container.llmClient` 冒充原始 provider。

### P1 iOS 凭据发布所有者与一次解析（验证中）

本片已改真实配置保存/选择、模型测试、prepared factory 与共享容器消费者：

- `IOSCloudCredentialOwner` 用固定 Keychain service 的唯一版本槽，SecItemAdd-only，
  不覆写已签发版本；私有 payload 绑定 profile、规范化 endpoint、协议和 upstream
  model。source 只保存显式 anonymous 或受限版本引用；缺失/错绑定不回退旧 modelID
  槽位或 selected.apiKey。旧槽位不读取、不删除，不做隐式迁移。
- `ModelInfo.redactedForPersistence` 成为纯值转换，不再写 Keychain 或吞写入失败。
  新凭据先写唯一版本，再用已有 Application receipt 的 expectedRevision CAS 发布
  profile。只有明确冲突/拒绝可删除本次新版本；超时/不确定确认保留以免删除已发布
  凭据。禁用状态保留，tombstone 在 staging 前拒绝，不因改 key 而复活档案。
- `upsertModelProfileCAS` 返回真实 committed revision，非客户端推算版本。
  `IOSResolvedCloudRunProfile` 一次读取保留 identity/revision/parametersJSON；prepared
  factory 保留该来源并校验 identity，尚不是可持久化完整 HostInvocation。
- ServiceContainer 的云端消费者先解析精确 profile/凭据和同版本参数，再做去重；
  失败清除旧 provider，不用全局旧 key 或混配全局模型参数。新 agent 继续消费自己
  的 prepared client；模型测试同样先解析档案。显式 anonymous 发送时不带空 Bearer。
- Provider 不再按 ModelScope 域名将明确模型 ID 偷换成另一名称；生成配置按行解析
  stop 时保留每项空白字符，避免 factory 在 provider 校验前篡改用户值。

本片隔离测试使用 fake Keychain + 真实 Application DB，无外网模型请求、不读写用户
真实凭据。统一 iOS 报告 `/tmp/nola-ios-credential-publication-20260928.xcresult` 尚在
执行。旧无引用 cloud 配置现在明确要求重新配置，不声称兼容旧槽位。跨端共享 secret
reference、endpoint query 私有重建、完整 HostInvocation 持久化和冷恢复 gate 接入
仍未完成，不能把版本化 Keychain 或本次 factory 元数据当作完整恢复能力。

OpenAI Docs 本轮核对了 [conversation state](https://developers.openai.com/api/docs/guides/conversation-state)：
provider conversation/output items 的续接不替代 Nola 对本地模型配置和凭据版本的
持久化所有权；这也是本片保持 provider 会话身份与本地 Run 身份分离的设计依据。

本片首轮实际构建通过，102 项中 101 通过、1 失败（0 skipped，56048 exit65）。
失败是新增 fixture 误以为 get_profile 返回 tombstone；该公开查询实际隐藏删除项。
已改为验证真实 get=nil 后以 expectedRevision=0 尝试同 ID 发布，必须被底层未过滤
tombstone 的 revision CAS 拒绝并仅清本次新 secret；另独立测试显式 tombstone
在 staging 前拒绝，不将构造的 tombstone 冒充数据库读取证据。同 102 项重跑路径
`/tmp/nola-ios-credential-publication-fixed-20260928.xcresult`，尚待最终结果。

最终同轮验证 102/102 通过，0 failed/skipped，原句柄 57535 exit0，实际 Xcode 编译
与模拟器执行完成。涵盖版本化发布、真实 CAS/tombstone、容器准备、同次档案来源、
协议字段及既有权限/投递身份回归。凭据复验发生于模型准备/容器重配置，不代表
已实现每次 HTTP 请求的即时撤销。未确认发布的孤立版本回收、共享 Host secret
reference、完整冷恢复及真实模型/设备测试仍未完成。

### P1 共享凭据来源合同与实际执行事实（接入中）

HostInvocation v7 / OutputAdmission v5 将 HTTP credential 与 endpoint query 的
来源设为必填：`none` 明确没有材料，`ephemeral` 只有独立取得的运行时材料，
`managed` 是受限的 owner/handle/revision/purpose 引用。摘要只用于比较，不能
取回材料；managed 结构合法也不授予读凭据权限。可信本地 resolver 仍须检查
owner、原路由、版本和当前撤销状态。没有 resolver 的平台不得签发假 managed。

CLI 的真实 route builder 现已输出来源；当前 Config 不携带可信 vault 引用，因此
非空 key/query 标 ephemeral，而非冒充已接入持久恢复。显式空 query (`?`) 仍保留
原摘要及 ephemeral，与无 query 区别。现有 clear tombstone 的禁止回退语义不变。
ephemeral 无法仅从快照重建，但不禁止调用者通过可信渠道重新提供原材料后通过
完整 exact 比对；当前 CLI 恢复仍属于后者，不是自动历史凭据重建。

iOS Prepared 保留实际创建 provider 的同一 Config 与同一次 profile 解析结果，
不再丢失执行事实后从 UI/default 反推。该对象包含密钥和完整 URL，仅限进程内，
不可 Codable、日志或导出。Native 仅记录 logical model ID，不称作权重内容身份。
这一步尚未将 HostInvocation 原子接入 canonical begin，也未开放冷恢复 gate。

本片首轮 Host 测试发现 Serde internally-tagged unit variants 会忽略额外字段，
`deny_unknown_fields` 单独声明不足。已改为私有空 struct wire variants 严格解码，
保留公开 None/Ephemeral 接口；原失败测试未削弱。修后 35/35 通过（77613 exit0）。
Application 真数据库查询新增自洽外层 digest 下的缺失/矛盾/错用途来源拒绝测试；
CLI 映射与 iOS 同源执行配置测试待本轮集成验收，不能沿用旧报告作为本轮通过证据。

本轮后续接线复核约束：Prepared → ChatManager → Coordinator → canonical begin
必须传递执行事实；Coordinator 在实际 permission_rules/catalog 形成后组装 binding，
不能让持久层重读 UI。已有 `AdmitHostOutput` query 可复用，iOS 还缺其调用边界；
explicit output 来自实际 Config，未知能力上限必须为 null，不能猜一个 cap。
`toolPlan.output.policyDigest` 不是完整执行政策摘要，不能直接替代它：原始工具
manifest、实际 permission rules、平台执行所有权仍须纳入稳定身份，刷新后的短期
receipt 应实时验真而非每次改掉 Run 身份。

还存在执行事实差异需在接线时消除：Native factory 的 catalog alias 解析结果未
回传，UI model ID 不能冒充实际加载目标；模型重试目前由 Coordinator 的 3 次尝试
实现，应提取为执行和 binding 共用的政策，不能在 Prepared 猜常量。本地文本工具
协议不等于 native tool calls；thinking 与 parallel/cache 受到每步 invocation 控制，
不能把 UI 开关直接当作实际请求或能力。这些事项仍是待实现项，不因 schema 已有
字段而计为完成。

OpenAI Docs 本轮复核 [Responses create](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)：
parallel tool calls 是独立请求选项，reasoning 为独立配置，max_output_tokens 同时
覆盖可见输出和推理 token。因此本项目设计上应区分 transport 可流式、模型能力、
用户生成意图及每步 Core 调用政策；不能由一个 supportsNativeEventStream 布尔量
推出整套能力。此为对当前 iOS 模板的整改依据，不表示本轮已修复全部能力接线。

本片 Rust 集成验证补充：Application 真数据库 10/10（17667 exit0）、恢复端口
13/13（45234 exit0）；CLI 输出准入 11/11（61136 exit0）及原绑定恢复 1/1（exit0）。
加 Host 35 项共 70 项通过，无 skipped。iOS 本轮实际 build/test 报告位于
`/tmp/nola-ios-execution-source-20260928.xcresult`，尚在执行。

iOS 本轮最终报告 13/13 通过，0 failed/skipped，25894 exit0，实际编译与模拟器
执行已结束。覆盖 Prepared 同源事实、本地 Run 配置及 provider generation fidelity；
不包含真实网络模型或 UI 自动化，亦不证明未接入的持久快照/冷恢复能力。

### P1 iOS 实际准备流程接入共享输出准入（验证中）

`IOSPreparedRunModel.prepare` 现必经 Application `AdmitHostOutput`，不是仅增加一个
未调用的 helper。HTTP 在创建 client 前，从同一 Config 提取 route、真实 token 字段
及 explicit output；共享结果的 route/tokens 必须与执行配置完全一致，否则拒绝
返回 prepared client。Native 由已构造 provider 的不可变 selectedModel 投影目录
身份，再进行同样准入；目录名不是权重版本，当前 LMModel.id 本身等于 name。
未知三类 token caps 仍为 null，不用会话 context/reserved output 伪造能力声明。

固定的 `IOSApplicationRuntime.admitHostOutput` 复用现有传输和回执身份校验，但不走
待确认 mutation 文件队列。只开放此已知纯 query，不使任意 mutation 绕开写回执。
测试检查成功与失败查询前后 DB/WAL/pending 文件字节不变。

HTTP route builder 用 percentEncodedQuery 计算摘要，保留 absent/empty query
区别，拒绝 userinfo/fragment。只对常见 scheme/host/default-port/empty-path 做
规范化并交 Rust 再验；不宣称 Foundation 与 WHATWG 全面等价。路径保留 percent
encoding，不在 admission 中追加 provider 路径，也不隐式 decode/re-encode。
当前 provider 实际 URL helper 对编码路径的处理仍需独立治理，这次保存的是准确
configured base 身份，不应把它误称作完整 HTTP request URL。

结果保留于 prepared.outputAdmission，完整 HostInvocation 与 policy/capability
快照仍待 Coordinator 原子提交接线；冷恢复 gate 仍不放行缺失的执行事实。
本轮实际 iOS 编译和模拟器测试报告 `/tmp/nola-ios-output-admission-20260928.xcresult`
正在执行，不能用上轮 13 项结果代替。

首轮 24616 exit65 停于 Swift 编译：默认准入 closure 缺 MainActor 标注，未执行
测试。已将 closure 的隔离域明确为 MainActor；同组重跑报告
`/tmp/nola-ios-output-admission-fixed-20260928.xcresult` 待最终结果。

修后本轮 21/21 通过，0 failed/skipped，35780 exit0；已核 xcresult 正式 summary。
包含固定 pure query、真实 Swift→Rust route 校验、prepared 配额/路由错配拒绝、
本地工厂身份与参数冻结及 provider generation 回归。此次改动已进入新 Run 的
实际准备路径，但 prepared 结果尚未完整组成持久 HostInvocation，不能据此宣称
冷恢复完成。下一接线必须同时消除 TextAgentModelClient 的固定能力模板，并让
Coordinator 的实际重试/工具执行政策与快照共享来源。

### P1 iOS 能力声明、Core 计划与执行门禁同源（验证中）

OpenAI Docs [Streaming API responses](https://developers.openai.com/api/docs/guides/streaming-responses)
定义的是 typed semantic events 的流式传输。结合本项目调用链，不能把实现了事件
解析协议当成模型支持工具/推理/缓存的证据。本轮新增 provider-neutral 的不可变
`LLMAgentCapabilities`，不依赖 Core JSON；OpenAIProvider 从其同一冻结 Config
声明执行事实，LocalProvider 从实际冻结 thinking/maxTokens 声明，不将本地文本
工具协议标作原生工具协议。未有完整证据的 parallel/continuity 自动续接仍关闭。

TextAgent 再将声明与实际 transport 能力、调用者 thinking 开关相交，形成非空、
按事实内容区分的 profile。所有未声明客户端也使用显式保守 profile，而非 nil。
原因是已查实 Core session 在 profile_id 为空时调用旧 resolver，其默认会开启
native tools/text-tool streaming；返回 nil 并不是此边界的安全降级。共享旧 resolver
在其他入口的全面消除仍待跨 Host 审查，不声称本片已改全局默认政策。

执行前检查 invocation 不得超出冻结能力；不允许因为强制工具指令而升级能力。
native=false 却收到 native tool delta 时拒绝，已声明 native 下的未知工具名称仍
原样交 Core 恢复，不加词表路由。event 和 legacy chat 的 thinking 均服从同一
Core plan，不再由 legacy 分支绕过；禁用时不将意外 reasoning 输出提升到结果。

输出限额与上下文预留分离：provider 的实际 maxTokens 进入 capability profile，
Coordinator 精确读取 Core effective_max_output_tokens 到 invocationPlan，TextAgent
检查它与冻结 provider 值一致。旧 reservedOutputTokens 仍是上下文估算，不能将
其至少 256 的启发式值冒充实际输出限额，也不能为了测试通过改写为 64。没有输出
事实的第三方适配器仍不能声称完整精确执行/恢复能力。

本片真实 Core→Coordinator→TextAgent→provider 输入矩阵覆盖未声明/工具关闭/
工具与思考开启，并验证实际 native tools、enable_thinking、64-token plan；另含
执行前拒绝、unexpected tool delta、legacy thinking 约束测试。当前统一 iOS 报告
`/tmp/nola-ios-execution-capabilities-20260928.xcresult` 正在执行，待正式结果。

本轮磁盘紧张时仅清除了已结束的四个可再生成 Rust 测试可执行文件（nola、
nola_application、effect_delivery_application_e2e、run_invocation_recovery_e2e
的上述已知 hash 产物）；保留所有源码、数据库和 xcresult。恢复方式是重跑对应
cargo test，不属于删除用户数据。外部正在执行的原生构建未中断。

本片首轮 4147 exit65 为新增 InvocationRecordingClient 测试代理缺 MainActor 标注，
停于测试编译；修后同组正式报告
`/tmp/nola-ios-execution-capabilities-fixed-20260928.xcresult` 76/76 通过，0 failed/skipped，
58338 exit0。实际编译与模拟器执行已完成，覆盖真实 Core 编排但 provider 为受控
fixture，并非真实联网模型。工具目录与取消/权限生命周期回归同组通过。

下一步完整 generation 快照须与 capability 一并解释，不能将 sampling_override
冒充模型 sampling 支持，或将 reasoning 显示/接收开关冒充远端实际 effort/budget。
Root 复核确认 Responses 对非零 penalties/seed/stop 当前会在请求前明确拒绝，
不是静默丢弃；但 Prepared 还可能先构造该无效 Config，因此应把同一生成参数
验证前移到 Run 准入并复用，避免运行创建后才失败。自定义 cache key/prefix 只有
digest 无法冷重建，仍待私有来源接入；完整 HostInvocation/policy 原子持久化仍未
完成，不能把本片能力传递验收当作全架构验收。

生成配置准入现复用 `OpenAIProvider.validateGenerationConfiguration`：
直接云配置工厂、ModelService 安装与 Chat/Responses 请求构造共用同一规则，
不支持的协议/参数组合在 Prepared 的输出准入之前拒绝。安装不再隐式复制云
密钥到旧 `nola.llm` 槽位；专门的 credential owner 负责发布。安装校验失败
保持原 provider，Container 的重配失败策略仍是显式 placeholder。

本片首轮模拟器正式结果
`/tmp/nola-ios-generation-admission-20260928.xcresult`：62/62 通过，
0 failed/skipped，33491 exit0。覆盖参数边界、真实隔离 DB/profile 到准备阶段、
安装替换语义及 canonical/model-selection 回归。准备 API 无创建副作用不代表
整个 ChatManager 失败时不产生空 conversation；这里只证明在 Run 创建前准入。

并行复核又发现旧 ModelTestView 在 Container 重配后仍以全局旧 key/default Chat
Config 覆盖 provider，现已删除这两处云重建分支。最终源码正在增量复测；旧调试
页的独立模式 state、未绑定执行的模型 Picker 与 legacy 配置表单仍需收敛，不能
把这次移除旁路宣称为整个调试界面重构完成。

最终增量复测编译完成，但 App 安装阶段报 `No space left on device`，不是
测试断言失败。最后旁路删除尚无运行验收，报告为
`/tmp/nola-ios-generation-admission-final-20260928.xcresult`。

并行 Mac 修复将 Workspace 的 memory 默认值及相关 userPreferences 改为读取
注入 repository 所属 store；新增双隔离 DB 的 global false/true × override
nil/true/false 矩阵与偏好 owner 隔离检查。三个文件语法/diff 检查通过，磁盘
不足未启动 Mac 构建。远端 Run 未传 memory override 的缺口仍未修复。

三档链路复核另确认两个待修边界：conditional sampling 在最终 effort=none
时可能因提前冻结 samplingOverride=false 而遗漏默认采样值，应在 canonical
reasoning resolution 后确定发送策略并验证实际 payload；会话 seed Optional
无法表达清除模型默认值，应改继承/清除/指定值语义，不能用合法 seed=0 代替。

后续验收已恢复：只清除了 `target/debug/deps` 下两份确认属于 Rust test harness
的旧 code_memory 可执行产物（192aa59675b2d97c、9d5f7305326719ca，约335 MiB），
可由 cargo test 重新生成；未删除产品可执行文件、源码、数据库或测试报告。
复用最后已编译的 iOS 源码产物执行 test-without-building，正式报告
`/tmp/nola-ios-generation-admission-recovered-20260928.xcresult` 为62/62通过，
0 failed/skipped，79906 exit0；补齐了上一条 ENOSPC 后未完成的验收。

Mac Runtime 现于 canonical reasoning selection 后，结合声明能力决定默认
采样是否发送，保留显式非法覆盖的拒绝行为；none+默认采样值不再被提前的
UI samplingOverride=false 丢弃。新增5项 Application→Host payload 测试，
覆盖none发送/high省略/显式不兼容拒绝/无条件支持/未知能力不提升。
与上一轮 memory owner 修复一起真实编译并运行，正式报告
`/tmp/nola-mac-sampling-memory-owner-20260928.xcresult` 为64/64通过，
0 failed/skipped，1988 exit0。该组证明原生payload与配置链路，不是联网模型
或跨进程最终HTTP回放验收。完整seed三态、remote memory override、完整
HostInvocation原子持久化与P5等目标仍未完成。

seed 三态实施已贯通 Domain/Application/Store/CLI/mac/Web：持久偏好为缺省继承、
clear对象、value+规范十进制i64字符串；最终模型请求仍是省略或精确整数。完整
settings写入现与patch共用modelOverrides校验，并拒绝持久seed:null，避免
全量写入绕过合同。Mac读取非法执行配置不再吞解码错误并恢复默认；稀疏跨端
metadata只补version/isPinned展示默认值，单会话load失败会抛错，批量load仍
记录并跳过不可读会话。不会删除或自动迁移旧数字seed配置。

本片真实验收：Domain2/2（33200）、Application集成两组共6/6（35504）、
Store3/3（25406）、CLI seed过滤5/5（98374），host_model_intent过滤7/7
（直接exit0，两个CLI过滤有重叠）。CLI包含实际本地HTTP抓包，覆盖省略、0、
负数、2^53以上及i64两端；Responses不兼容默认seed在session clear后可运行。
Web定向8/8含组件编译，完整svelte-check 0 errors/0 warnings（84481）。

Mac首轮报告 `/tmp/nola-mac-seed-contract-20260928.xcresult` 72/72通过，
但新增Decodable extension有MainActor隔离警告；补nonisolated后正式重跑
`/tmp/nola-mac-seed-contract-final-20260928.xcresult` 仍72/72通过，
0 failed/skipped，40054 exit0，新增隔离警告已消除。覆盖typed codec、隔离DB
重开/错误不降级、远端Foundation JSON与本地Runtime payload、三档与记忆回归。

磁盘不足时仅删除了确认fingerprint为test-bin-nola的两份旧Rust测试可执行产物
`target/debug/deps/nola-85d26f111070b1ff`、`nola-05f1fae23b4aa928`（约472MiB）；
可cargo test重建，未删产品可执行文件、用户数据、源码或测试报告。
此处仍是受控fixture验证，不是所有平台真实模型/交互验收；完整HostInvocation、
远端memory policy与P5等整体目标继续保持未完成。

共享Application会话单元回归另跑 `cargo test -p nola_application --lib conversation_`，
25/25通过，90897 exit0（日志 `/tmp/nola-seed-application-unit-20260928.log`），
补验完整settings准入变化对已有会话用例的影响。

记忆审查确认新的真实旁路：Core工具/recall关闭不等于手工Memory组件已被禁止。
本片在Core start_turn最前按冻结MemoryPolicy统一准入，在创建持久Turn上下文及
模型请求之前移除禁用的跨会话Memory；新增SessionMemory明确当前会话摘要，
预算、引用包装与被新summary替换时的消重规则均保持。决策只看类型，不看ID
或自然语言。UserPreference与PersonalIntelligence不被长期记忆开关替代授权。

生产端Web conversation.session_memory与iOS会话摘要、Android compactedEntries
改为session_memory；iOS三个durable记忆生产分支统一为Memory，学习出的偏好
不再伪装显式用户指令。普通用户偏好保持原类型。iOS/Android指标区分会话与
跨会话记忆。Core真实运行5/5通过（17498），预算/summary替代1/1通过（9971）；
客户端编译/回归结果待本片后续记录。远端Bool未传递、当前Host撤权检查和完整
HostInvocation政策绑定尚未实施，本片不能证明“远端禁用长期记忆”已可用。

本片后续正式验收：CLI Web类型分类1/1通过，59261 exit0；iOS重编共享Core
并运行模拟器报告 `/tmp/nola-ios-memory-context-20260928.xcresult`，31/31通过，
0 failed/skipped，1894 exit0；AndroidContextCoordinatorTest定向JVM回归5/5通过，
0 failed/skipped（25895 exit0），并非设备/JNI端到端或联网模型验收。
为模拟器腾空间仅移除本轮刚完成的可重建CLI测试产物
`target/debug/deps/nola-2d6063be522fed79`（约241MiB），保留日志、产品程序和数据。

### 远端记忆请求与恢复准入（2026-09-28，续）

三路并行：CLI 请求解析、Mac 发送冻结与 HTTP、只读权限审查；主代理整合
HostInvocation 摘要及 checkpoint 恢复。客户端只提交 memory_enabled 意图，
Host 独立权限上限不被请求提升；Core 读取不可从 JSON 伪造的准入结果。
新 Run 清除旧准入，恢复旧 Run 保留 checkpoint 策略并检查当前 Host 开关。
完整策略进入 execution_policy_sha256。详见 MEMORY_EXECUTION_POLICY_CONTRACT.md。

CLI `CARGO_INCREMENTAL=0 cargo test -p nola_cli --bin nola memory_ --quiet`
33/33 通过（83629 exit0），日志 `/tmp/nola-remote-memory-admission-20260928-r2.log`。
首次新增测试误用不存在字段 resident_limit，编译失败；改为实际 search_limit
后重跑通过。Mac 定向验收另记。不代表每个 effect 的实时撤权已实现，也不代表
真实联网模型、跨进程完整恢复或总体 P0–P6 验收完成。

Mac 本轮定向构建 4568 exit65：共享 Rust archive 写入时报 `No space left on
device`，测试未执行，不能计为通过。日志 `/tmp/nola-mac-remote-memory-20260928.log`，
结果包 `/tmp/nola-mac-remote-memory-20260928.xcresult`。未清理用户数据。

### 逐 effect 记忆权限准入（2026-09-28，续）

入口选择为 CliAgentHostDriver::dispatch，不在外围 execute 循环或各工具里重复
检查。冻结 policy 与当前 Host 开关上限在 ExecutionStarted 前比较；不可重试
拒绝经响应提交链处理，Cancel/CancelTask 保持可用。配置化 batch 不绕过此入口。
这不等价于中断已开始的流，也未证明所有平台均具备实时撤权。

恢复测试扩展真实 loopback provider + 隔离持久化 Store：旧 Run 的 false 不随
恢复请求 true 改变；同会话新 Run 重新准入 true，但不提升 Host 默认关闭的
自动提取。另增派发前拒绝、无 delivery batch、取消仍有效的 driver 回归。

为重跑回收已确认无进程使用的可重建产物：CLI 测试 executable
`target/debug/deps/nola-2d6063be522fed79`，以及 Mac runtime deps 下的
`libnola_core.rlib`、`libnola_core.a`、`libnola_core.dylib`；没有删除产品程序、
数据库或验收日志。初次新增 dispatch 测试缺少 Arc/HostEffectId imports，已补齐。

真实恢复回归发现并修复第二个状态真源问题：合法 idle 配置提交后，
CommittedCoreRuntime 仍拿旧内存配置 restore，memory false→true 报 mismatch。
现在从 Application 已提交 checkpoint 同步完整 SessionConfiguration 六字段及
permission rules；保留 Core 严格 restore 校验，不接收客户端伪造 checkpoint。
恢复失败时内存配置/catalog 的全原子切换仍可进一步收紧，未作为已完成能力。

验收：memory_ 34/34（16384 exit0），真实 HTTP/Store 多次恢复+新 Run 1/1
（8808 exit0）；完整 permission_recovery_entrypoint_tests 8/8、
effect_delivery_start_barrier_tests 12/12、MCP retained-process 事务1/1
（92988 exit0）。日志分别为 `/tmp/nola-memory-dispatch-20260928-r2.log`、
`/tmp/nola-memory-real-recovery-20260928-r2.log`、
`/tmp/nola-memory-recovery-suite-20260928.log`、
`/tmp/nola-memory-delivery-suite-20260928.log`、`/tmp/nola-memory-mcp-20260928.log`。
这些测试未证明撤权后 canonical Run 的完整终态收束或已开始 effect 的实时取消。
CLI 测试产物跑完后再次删除以腾出 Mac 构建空间，测试证据保留。

Mac 重跑 29146 exit65：本次 core archive 已生成，但 mobile bindings 动态库
链接仍因 errno28/No space left on device 失败，定向测试未执行。证据为
`/tmp/nola-mac-remote-memory-20260928-r2.log` 及同名 `.xcresult`；失败后可用
空间约189MiB。不能用 CLI 通过替代 Mac 验收，也未删除历史产品 binary 腾空间。

### 恢复缓存原子替换与补验（2026-09-28，续）

已收紧上一批记录的恢复失败边界：prepare_committed_catalog 只构造候选目录，
恢复先准备候选 config/catalog 并通过 Core 校验，随后提交 live catalog、config、
checkpoint、snapshot。保留进程独有的未激活工具目录合并语义；未放宽 session
或 Host 身份验证。新增故障注入证明错误 session 恢复后四份状态完全不变；
合法 Core 配置/目录提交可同步生效。

CLI permission_recovery_entrypoint_tests 9/9通过，99312 exit0；
日志 `/tmp/nola-atomic-restore-20260928.log`。没有据此宣称全端原子恢复完成。
尚未专测 foreign-host prepare 失败与未激活工具保留的负向 fixture。

为继续验证，逐项核对 Cargo test-integration-test 指纹后，精确删除34个Sep14
nola_application旧集成测试 executable，每项均存在更新副本，合计约2.060GiB。
仅 target/debug/deps 可重新编译的测试产物；未删除产品、测试日志、源码或数据库。
Mac 第三次补验使用独立结果包 `/tmp/nola-mac-remote-memory-20260928-r3.xcresult`。

第三次 95932 exit65：构建成功，26 tests中21 passed/5 failed/0 skipped，
7条断言均来自新增 HTTP memory 断言直接读取加密信封外层，误把字段视作缺失。
不是生产关闭加密来迁就测试：修正为对实际捕获信封用 fixture key/AAD 解密后
断言 bool，随后重新运行整组。第三次失败报告保留，不替换为成功证据。

第四次 Mac 8393 exit0，26/26 passed、0 failed/skipped；xcresulttool确认
`/tmp/nola-mac-remote-memory-20260928-r4.xcresult`。覆盖四个完整suite：
MacConversationMemoryOverrideTests、MacRunSubmissionPreparationTests、
MacRemoteCancellationReceiptTests、MacRemoteModelIntentTests。实际HTTP请求认证
解密后验证false/true；生产加密未改变。发送期间UI变化不改冻结值，resume不带
新memory覆盖；取消与投影归属相关旧回归一起通过。仍非截图/AX或外网模型验收。

### Host 记忆准入模块与拒绝持久化（2026-09-28，续）

抽取 host_memory_admission.rs，集中独立 Host 配置、客户端限制、冻结/恢复和
当前权限上限校验。native bridge 与 driver 直接依赖此模块，不再反向引用
core_agent_runtime 的策略 helper；运行编排仅调用新模块，不保留兼容包装。
两个纯策略测试同步迁移，两个共享 main 的 CLI executable 均注册同一模块。

Application 新增“派发前拒绝→数据库重开→迟到开始响应拒绝”回归，30350 exit0，
1/1通过；日志 `/tmp/nola-rejection-receipt-20260928-r2.log`。最初测试把 settlement
预期成 Failed，实际为更准确的 RejectedBeforeExecution，修正测试而非降级
生产数据模型。model attempt 保留失败原因，旧 delivery 不可重新启动。
CLI 真正 dispatcher→canonical Store→Run 终态回归另行验证。

CLI memory_ 35/35通过，90528 exit0（含真实派发前撤权收束）；日志
`/tmp/nola-memory-module-20260928-r2.log`。测试先提交 enabled Run，再修改独立
Host上限，真实driver拒绝，监听器确认零模型连接，inbox有拒绝无开始记录，
activeRun及pending effects/outbox均清空。首轮新增测试误调用HostEffectJournal
不存在的is_empty，改用len后完整重跑。此证据不推广为任意工具执行中撤权。

补验：Application effect_delivery_application_e2e全7/7、CLI恢复套件10/10
通过（89819中的前两条命令；对应日志 `/tmp/nola-rejection-receipt-suite-20260928.log`
与 `/tmp/nola-memory-module-recovery-20260928.log`）。第三条最初误用underscore
binary名而未执行检查，改为真实target `nola-agent-host` 后cargo check通过
（88328 exit0），日志 `/tmp/nola-memory-module-host-check-20260928-r2.log`。
新增独立模块已被生产Host编译验证；不是仅测试配置下存在的代码。

### 权限等待期间撤权的真实副作用验证（2026-09-28）

标准Core RequestPermission返回只提交PermissionResolved，不在回调里直接
执行工具；后继ExecuteTool经过统一dispatch重新准入。新增实际HTTP/file fixture
在等待批准时撤销Host权限并返回Allow，证明目标文件不存在、Run结束、pending
permissions/effects和activeRun清空，模型请求数停在首个工具请求，不继续校验/
候选/审查调用。没有为已有正确边界额外增加另一套权限决策。

CLI完整permission_recovery_entrypoint_tests 11/11通过，38350 exit0；日志
`/tmp/nola-permission-memory-revocation-20260928-r2.log`。首编新增断言使用不存在
的HostEffectJournal.is_empty，改用len后重跑。这个验证不覆盖已经运行的delegated
worker、device bridge内部批准恢复及工具副作用中途撤销，不能宣称全端实时撤权。

### P5 原始正文 reader 前置统一（2026-09-28）

新增 [Canonical流式正文合同](../../软件架构设计/模块/13-客户端投影/CANONICAL_STREAMING_TEXT_CONTRACT.md)，明确私有
字段head与公开journal引用分离，以及writer/reader/replica必须一起迁移的门槛。
本批没有打开新Chunked writer，不把既有未接线journal publication声称为生产链。

生产改动：conversation_read_repository 的正文不再SQL裁剪后另补sealed字段，
统一由CanonicalFieldReader读取；原JSON裁剪仍在投影层，保留既有行为。
transcript_search删除inline+sealed两套候选流，单一路径按canonical顺序遍历
metadata并调用reader.find_literal，收够结果立即停止读取正文；Inline的借用UDF
单次匹配与有界SQL截取放在reader内部，Sealed复用有界range搜索。
未改摘要、branch或原始JSON消费者，不宣称全部读取路径已迁移。

新增Inline/Sealed Unicode tail、digest、原JSON保真/限制及冲突拒绝测试3/3
（17475 exit0）；完整Store lib回归198/198通过（73732 exit0），含10M正文搜索、
2000 Part搜索和实际生产循环每个候选只扫描一次的仪表断言。
日志 `/tmp/nola-canonical-reader-hydration-20260928.log` 与
`/tmp/nola-canonical-reader-store-20260928.log`。复制/导出集成另行记录。

Store集成补验51245 exit0：ledger_e2e 17/17、structured_field_export_e2e 3/3、
sync_replica_e2e 1/1、turn_append_sync_e2e 5/5，共26/26通过；日志
`/tmp/nola-canonical-reader-integration-20260928.log`。验证的是现有Inline/Sealed
生产行为及复制不回退，尚不存在Chunked字段端到端或2/8/32MiB追加计量证据。

### P5 摘要与用户轮数的字段真源（2026-09-28，实施中）

conversation_summary_store取消直接截取content_text槽，改为固定快照内选择首个
可见user Part，再由CanonicalFieldReader读取最多512字符原文前缀。无Part仍为
None，引用冲突明确失败；已隐藏首条消息不再泄漏到摘要。message_count等已有
账本metadata语义未在本片改变。

conversation_read_store的用户轮数分类改为原始content_json读取；复用结构化
token reader，新增不保留元素正文的完整数组/对象成员遍历，避免前64项展示
提取器漏掉后面的tool_result。范围读取需允许不足4-byte的短token，通过最小
合法range再按已验证token边界裁剪，不能将{}或合法短尾当错误。
重复目标键/损坏JSON从旧SQL默认普通用户收紧为明确错误，引用错误不吞掉。
这不是整个tail窗口对deferred JSON已兼容；现有hydration限制仍需下一批迁移。

本批Store lib回归204/204通过，日志
`/tmp/nola-summary-context-field-reader-20260928.log`；复制/导出集成26/26通过
（57311 exit0：ledger 17、structured field export 3、sync replica 1、turn append sync 5），
日志 `/tmp/nola-summary-context-integration-20260928.log`。这些结果不包含新Chunked
writer或全部JSON hydration迁移。后续将JSON读取、branch复制及边界测试拆分并行，
共享源码按文件划定责任，最终编译与集成测试串行执行。

并行补测后完整Store lib为207/207通过（92747 exit0），日志
`/tmp/nola-summary-context-parallel-regression-20260928.log`。新增覆盖UTF-8跨chunk、
短token/转义键、对象直接成员与深层成员区别、隐藏损坏引用及恢复后的拒绝、
已识别工具结果后仍拒绝后续Part歧义；测试fixture也按UTF-8边界拆chunk。

并行审计形成两个未实施的后续方案：
[JSON hydration迁移](CANONICAL_JSON_HYDRATION_PLAN.md)明确原始字节保真、
Absent/Complete/Deferred、字节预算及review最终额度；
[branch双字段迁移](CANONICAL_BRANCH_FIELD_PLAN.md)明确publication、replica与
derivation/archive必须共同验收后才能解除text-only限制。两份方案不是生产完成证据。

### P5 JSON hydration 与延迟元数据显式化（2026-09-28，本批验收通过）

三个并行任务分别实现原始JSON reader、bounded_message读取统一、边界回归；root
接通repository与独立review投影模块。read_json使用Absent/Complete/Deferred私有
结果，None预算显式无界、Some(0)为零预算；Inline单次SQL读取，Sealed按范围组装，
完整JSON原始字节/大整数/转义/空白不重编码。Deferred只证明字段绑定有效，不声称
未读取chunks完整，完整读取遇缺chunk仍错误。

repository移除inline JSON SQL真源，仅此消费者取消text-only门禁。JSON预算由
字符数明确收紧为UTF-8字节；review只替换已验证的text token span，最终超预算
延迟，重复review/text键报错；投影与延迟均移除原始digest。其余旧门禁未开放。

集成时发现Option<String>会在显示层丢失“缺失/未加载”的区别：CLI把缺review
当作无审查，可能显示未知状态草稿。Store/Application读DTO新增必需
content_json_deferred（非数据库字段），完整读取false、预算省略true；CLI不把
该状态当approved正文，也不把provider预览解析成完整envelope或用户输入。
空的延迟预览同样不能被推断为工具事件，相关回归首跑暴露此旧判定后已修正。

已通过Store lib217/217（49078 exit0，含最终DTO变更），Application lib165/165
（51912 exit0）、historical_projection_port_e2e 2/2（99818 exit0）。对应日志：
`/tmp/nola-json-hydration-deferred-store-20260928.log`、
`/tmp/nola-json-hydration-application-20260928.log`、
`/tmp/nola-json-hydration-history-port-20260928.log`。

DTO扩展前Store五组集成33/33通过（48614 exit0，append7、ledger17、structured3、
sync1、turn_append5），日志 `/tmp/nola-json-hydration-integration-20260928.log`；
不能将此前结果当作最终DTO后的重跑。CLI session::tests19/19、interactive_session
24/24及approved review1/1已过，新增deferred重跑和大历史恢复验证另行记录。
此批仍是读取前置整改，不证明新writer、双字段publication/replica或branch已完成。

最终DTO之后Store五组集成再次33/33通过（37818 exit0），日志
`/tmp/nola-json-hydration-deferred-integration-20260928.log`。CLI新增deferred回归
2/2重跑通过（35783 exit0），interactive_session最终再跑24/24；两个千万字符
会话恢复/切换1/1（91446 exit0）与万条工具事件恢复1/1通过。日志分别为
`/tmp/nola-json-hydration-cli-deferred-20260928-r2.log`、
`/tmp/nola-json-hydration-cli-interactive-20260928-r2.log`、
`/tmp/nola-json-hydration-cli-large-history-20260928.log`、
`/tmp/nola-json-hydration-cli-tool-history-20260928.log`。
Application historical_window_budget/cutover两组共4/4通过（3370 exit0），日志
`/tmp/nola-json-hydration-history-windows-20260928.log`，覆盖原始历史预算与语义
加载路径不依赖显示DTO中的JSON值。

尚未完成：branch双字段publication/replica闭环、Chunked writer与追加计量、全端
最终验收。CLI预算省略内容目前保守不显示，显式未加载提示/按需加载交互仍需完善；
大历史测试证明已有恢复边界，没有证明所有省略正文都可在当前界面中展开。

### P5 branch / replica / archive 双字段接线（2026-09-28，验收中）

branch按CanonicalFieldReader分别复制ContentText/ContentJson；小Inline保留原字节，
大字段或Sealed使用有界range建立新owner对象，content ID区分body/body-json。
Sealed源先完整校验声明chunk_count、连续布局及JSON语法，避免仅range可读却把
损坏源重新封存为合法目标。目标append、derivation、lineage、sync仍同一写事务。

append publication及其replica manifest使用Kernel统一字段绑定准入，绑定field
不再写死text。原本已field-aware的canonical_text_replica chunk/seal完整性校验
继续复用，没有另建JSON复制协议。Archive Store、Application typed上传manifest、
upload retirement和Mac归档写入器一起迁移；普通transcript文本上传仍保留专门合同。
ArchivePartUpload.field为必需字段，无默认推断；Mac同Part的两个upload ID含field。

预检与布局模型拆开：Kernel BodyBindingDeclaration只描述owner/field/object身份，
不伪造CanonicalBodyReference的chunk_count；实际对象准备后才检查真实布局。
这样>32KiB正常上传不会因错误的“单块预检引用”被拒绝。结构绑定先全量准入，
准备阶段可保留无可见消息的staging对象，最终message发布仍由命令UoW原子提交。

初步验收：Store228/228通过（5670 exit0），Kernel117/117（84915 exit0），
Application archive_restore_staging/receipt/runtime、archive_derivation_receipt及
branch_transaction五组共12/12（95490 exit0）。日志：
`/tmp/nola-dual-field-store-20260928-r3.log`、
`/tmp/nola-dual-field-kernel-20260928.log`、
`/tmp/nola-dual-field-application-20260928.log`。
最终新增sealed源几何回归后 Store lib **229/229** 再次通过，0 failed/ignored，
日志 `/tmp/nola-dual-field-store-20260928-final.log`。Text/JSON 各验证声明
chunk_count 错误及声明范围外多余chunk：即使范围读取仍成功，branch必须拒绝，
目标不可见且对象/消息/同步等原子操作计数不变。不能通过重新复制把损坏布局
“修成”新合法引用。此前 Kernel117 与 Application五组12的日志也已核实；
没有将它们冒称为最后 geometry 测试之后的重跑。

Mac真实备份往返与上传器定向 **10/10** 通过（session1607 exit0），两组为
MacCanonicalExportTests7 与 MacTranscriptBodyUploaderTests3。日志
`/tmp/nola-dual-field-mac-20260928.log` 为 `TEST SUCCEEDED`；xcresult
`/tmp/nola-dual-field-mac-20260928.xcresult` summary 核实10 passed、0 failed、
0 skipped。此为两组定向，不代表全部Mac验收。
Service `archive_restore_process_e2e` **2/2** 通过（session67806 exit0），实际进程
边界日志 `/tmp/nola-dual-field-service-20260928.log`；不以Store或Mac测试替代。
Store五组integration补验 **17/17** 通过（session32931 exit0，7+3+1+1+5），
日志 `/tmp/nola-dual-field-integration-20260928.log`；本轮为append_messages_e2e、
structured_field_export_e2e、structured_json_validation_bounded、sync_replica_e2e、
turn_append_sync_e2e，不是此前hydration的33项组合。命令为
`CARGO_INCREMENTAL=0 cargo test -p nola_store --test append_messages_e2e --test turn_append_sync_e2e --test sync_replica_e2e --test structured_field_export_e2e --test structured_json_validation_bounded --quiet`。
本批完成共享双字段接线及上述验收，不标P5整阶段完成；Chunked可追加writer、
增量streaming与全端最终验收仍待后续。

首轮编译测试误用了不存在的PartBodyField.as_str，改为显式enum映射；其次三项
失败分别为旧JSON一律拒绝测试、使用旧delivery cursor重投整个page的错误fixture、
将snippet起点误当match起点。均按实际字段/传输/搜索合同修正测试，没有放宽
ownership、单调cursor或source完整性准入。JSON retirement改由真实字段/字节
一致性验证测试覆盖，不继续把JSON类型本身当拒绝理由。

### P5 增量正文合同及真实模块拆分（2026-09-28，生产writer尚未切换）

按[流式正文合同](../../软件架构设计/模块/13-客户端投影/CANONICAL_STREAMING_TEXT_CONTRACT.md)继续实施，不能把上一批
双字段Inline/Sealed接线等同于生产流式追加完成。本批重新获取OpenAI官方流式
文档，并查看本地opencode的text-start/delta/end实现；事件分层依据与链接已写入合同。

三个并行子任务完成Domain纯流式契约、Application生产投影模块拆分、Store生产
mutation模块拆分。新`canonical_part_projection.rs`供原两处planner及其它已有调用
复用，`canonical_part_mutation.rs`供source和replica共用；旧SQL与权限语义保持，
因此旧prefix哈希及全文拼接成本仍存在，未宣称本轮已经修复端到端性能。

Domain新增完整owner/head及Declare/Append/Replace/Close，单块32KiB、精确head
CAS、独立哈希域、代际替换、关闭后拒写、未知字段拒绝。Store新增候选chunk/head/
manifest事务实现，仅`#[cfg(test)]`加载，schema不进入生产初始化；调用者负责整个
transition回滚，不由字段模块单独commit。复核纠正了“只承认Run初始assistant”
的错误约束，改为核对消息实际run/conversation/Part关系，允许同Run合法后续消息。

已核实验收（不是全端最终验收）：

- Domain初版流式合同5/5，session8332 exit0，日志
  `/tmp/nola-stream-contract-domain-20260928.log`。之后发现head最低revision形状
  尚需补强，此初版结果不能替代补强后的重跑。
- Application lib **168/168**，session67558 exit0，日志
  `/tmp/nola-stream-modules-application-20260928.log`，含新增3项模块投影测试。
- Store lib **234/234**，另ledger17、conversation_sync1、remote_content_sync4、
  turn_append_sync5共**27/27**，session31498 exit0，日志
  `/tmp/nola-stream-modules-store-final-20260928.log`。新增候选事务5项覆盖Unicode、
  reopen、代际替换、关闭后拒写、owner冲突、合法successor、manifest写入后的
  chunk/head SQL故障与外层回滚、旧backing冲突；不把它们称为生产writer接线测试。

剩余硬门槛：Chunked原始字段reader、缺块/损坏校验、Close封存及配额、metadata-only
与显式Replace生产策略、source/replica同事务接线、真实Application delta链路，
以及2/8/32MiB实际I/O证据。Core全checkpoint序列化成本另计，当前没有证明其线性。

最后head形状补强后的验收：Domain全lib **73/73**（含6项stream契约），session96664
exit0，`/tmp/nola-stream-contract-domain-final-20260928.log`；Store全lib再次
**234/234**，session43575 exit0，`/tmp/nola-stream-modules-store-r2-20260928.log`。
两次均0 failed/ignored。Application168与integration27为此前同批拆分验收，未冒称
是此次纯head形状补强后的重跑。所有本批文件限定diff检查通过；本批未改客户端UI。

### P5 Chunked原始正文读取闭环（2026-09-28，writer仍未生产启用）

本批新增`canonical_stream_read`，由`published_part_body`和`CanonicalFieldReader`
显式解析Chunked backing。已存在但损坏的stream不得fallback成空inline；JSON
semantic读取明确拒绝text stream。生产初始化仍不创建stream表，测试只在隔离库
安装候选schema；没有本地用户数据库迁移或清理。

range/resolve/tail/完整审计均固定快照，验证完整持久owner和visibility。每个被访问
块重算其局部manifest，Close与branch再审计全部祖先链。局部读取不宣称验证任意
自洽数据库分叉。Branch包括小正文转inline的分支都先完整审计，不允许损坏源通过
复制被重新认证为有效目标。候选writer Close新增完整审计，Append仍不扫描旧全文。

同时修复了读取设计中的两处线性扫描风险：额外尾块用两个独立索引范围查询，
不用单个OR遍历整代；Chunked的tail投影按块索引逆序收够字符预算即停，不从第一个
块读起。搜索抽出真正backing无关的有界reader接口，不构造假Sealed引用。

已核实验收：

- `CARGO_INCREMENTAL=0 cargo test -p nola_store --lib --quiet`，session74198 exit0，
  **248/248**，`/tmp/nola-chunked-read-store-20260928.log`。新增14项为read7、真实
  consumer5、Close损坏前块拒绝1、通用搜索range验证1。
- 六组Store integration **31/31**，session51366 exit0，日志
  `/tmp/nola-chunked-read-integration-20260928.log`：conversation_sync1、ledger17、
  remote_content_sync4、structured_field_export3、structured_json_validation_bounded1、
  turn_append_sync5。使用对应六个`--test`的cargo test，CARGO_INCREMENTAL=0。
- consumer实际执行writer→Close→冷重开→hydrate/Unicode tail/prefix/range/search、
  hidden/wrongowner拒绝、export manifest及17字节有界读取、branch独立复制/删源后
  重开，以及同长篡改source后分支整事务拒绝，不仅测试helper字符串拼接。
- `EXPLAIN QUERY PLAN`验证chunk_index和offset_bytes分别走索引范围；不是
  2/8/32MiB端到端实际I/O测量，也没有声称全checkpoint成本已消除。

Application本轮lib **168/168**、五组历史窗口/branch/archive定向 **14/14**通过，
session36835 exit0，`/tmp/nola-chunked-read-application-20260928.log`。命令包含
`--lib --test historical_projection_port_e2e --test historical_window_budget_e2e
--test historical_window_cutover_e2e --test branch_transaction_e2e
--test archive_restore_staging_e2e`，CARGO_INCREMENTAL=0。所有本批已改文件
限定diff检查通过；未运行Mac全套或真实模型端到端，不能借这些Rust测试宣称它们通过。
真实生产writer规划、StorePort head查询、同批delta overlay、metadata-only/
Replace语义、replica及事务receipt重放、Close封存/配额和性能规模验收仍是后续硬门槛。

### P5 生产增量正文提交接线（2026-09-28，续接归属定向验收通过）

本批在前述reader闭环之后打开Store生产接口：schema31保留既有会话的增表升级，
stream模块不再test-only；`PartMutation::Stream`、不含正文的`Metadata`进入既有
source/replica原子transition。普通Upsert/旧prefix Append不得覆盖已存在stream，
Metadata不得替代生命周期Close，Part Close从原始reader核验最终正文digest。
当前代逻辑字段上限256MiB，写入前常量比较，不每次SUM旧块；整个Store含旧generation
的物理配额/回收仍未完成，不能把字段上限说成全局存储预算。

`RunEffectRepository`新增必需`load_canonical_text_state`，区分Missing、Existing与
Stream；4个非SQLite测试适配器显式实现，无“默认返回Missing”的隐藏降级。
查询和读取使用canonical消息run身份，不能依赖不复制的本机runtime runs表。
source提交仍由真实Run前置校验，replica由已验证消息publication校验；没有创建假Run。

Application两个实际事件循环已接transaction-local planner，并包含placeholder与
complete_assistant_turn；同Part只查一次head，同批delta通过overlay连续推进，UTF-8
按32KiB分块。Text的ensure_message不再每delta建立累计正文initial_parts，metadata
构造不克隆累计text。已有非空非stream正文保持原语义路径，不自动用空head覆盖。

已核实结果：

- Domain **75/75**、Kernel **119/119**，session26154 exit0，日志
  `/tmp/nola-stream-production-contracts-r2-20260928.log`。首轮测试发现internally
  tagged unit Missing并不拒绝未知键，已改为严格adjacent type/value编码后重跑。
- Store最终lib **252/252**，session39662 exit0，日志
  `/tmp/nola-stream-production-store-final-20260928.log`，含真实commit→无runtime Run
  的replica→重放→重开、owner/旧writer/Close拒绝回滚、真正schema30升级保留会话、
  字段配额边界。旧测试改攻击canonical message owner，不再把runtime Run变动当
  正文归属变化；跨run/conversation攻击仍须拒绝。
- Store五组integration **28/28**，session66912 exit0，日志
  `/tmp/nola-stream-production-store-integration-20260928.log`，为conversation_sync1、
  ledger17、remote_content_sync4、sync_replica1、turn_append_sync5；此结果在最后
  配额helper添加之前，之后全lib已重跑。
- Application初轮lib **173/173**，但新增真实流程test失败，session49978 exit101，
  `/tmp/nola-stream-production-application-20260928.log`，因此不记该批全部通过。
  重复70KiB z触发真实Core loop guard重试，不能把AwaitingModel当最终完成；fixture
  改为不同记录，并保留独立真实discard回归。同时暴露TextReplaced/Created/review
  可携带is_complete而无额外Completed的生产收口遗漏，已统一修复。
- Application重跑 **174/174** lib及四组integration **11/11**，session6377 exit0，
  `/tmp/nola-stream-production-application-r2-20260928.log`：真实stream transition2、
  historical projection2、idle checkpoint2、run effect port5。覆盖真实批量delta、
  冷重启续写、receipt重放、最终关闭，以及loop guard丢弃后继续请求模型。
- 扩展生命周期验收尚未全通过：session76967 exit101，
  `/tmp/nola-stream-production-lifecycle-20260928.log`。conversation generation3、
  model generation8、run cancellation6通过；turn transaction为14/15。
  内部续接复用assistant并改写`messages.run_id`，与旧正文不可变的stream owner冲突，
  使历史正文读取失败。必须区分当前执行Run与正文创建Run的权威关联；不得改写旧head、
  取消归属校验或借用replica不存在的runtime runs来消除报错。

续接归属修复与最终重跑：

- schema32增表保存可信canonical message-run续接关系和准确的本机Run初始Part。
  source/replica在同一续接事务内登记；不改旧stream head、不伪造replica runtime Run，
  历史读取权不授予旧Run写权。内部Run无placeholder，Application不再误关旧Part。
- 可信续接前验证已发布Sealed字段，续接后仍要求Part完成、引用归属与inline互斥。
  text/JSON读取覆盖真实续接及冷重开；无合法续接的queued消息不能获得发布资格。
- Store lib **258/258**、五组integration **28/28**，session52818 exit0，
  `/tmp/nola-resumption-store-20260928.log`。同前五个integration目标，新增覆盖
  真实schema31升级、准确占位归属、历史正文续接、同步重放和事务后段失败回滚。
- Application lib **174/174**、八组integration **43/43**，session68742 exit0，
  `/tmp/nola-resumption-application-20260928.log`。目标为turn_transaction_e2e15、
  run_cancellation_e2e6、model_generation_contract8、conversation_generation_contract3、
  canonical_stream_transition_e2e2、run_effect_port_e2e5、idle_checkpoint_port_e2e2、
  historical_projection_port_e2e2。此前失败的内部续接还增加第二Run实际正文写入、
  完成与再次冷重开，确认旧head不变、新head正确关闭。该扩展使用真实Store transition
  和Application完成入口，不冒充网络真实模型测试。命令均使用CARGO_INCREMENTAL=0。

本批共 **503** 项通过。仍需更多全客户端/真实模型验收、2/8/32MiB实际I/O及checkpoint
成本测量、全Store物理配额/回收；旧版正在执行的Run恢复未用“猜首Part”兼容，需独立
验证。不得将这些定向结果当作整体P5或全部架构整改完成。

### P5 规模实测与借用正文边界（2026-09-28）

继续按真实生产消费者排查，不将Store分块完成当作全链路完成：

- `CanonicalPartProjection`将身份映射与正文所有权分开，两个实际Application reducer
  都借用Text正文；删除`canonical_part_for_run`每次克隆全Part的旧入口。alias只作用
  于有界metadata，终态或旧非stream路径才物化完整Part。8MiB用例验证借用同一正文
  地址、alias及严格owner、完成JSON/digest；不是简单删掉归属检查。
- 诊断事件Text/Reasoning/Compaction直接从借用字段构造预览；不再先克隆整个事件和
  正文再截断。超限探测最多扫描budget+1个字符，保留Unicode前后缀及原始事件不变。
  Tool JSON、Core checkpoint、累计事件envelope仍有独立成本，不声称整体已线性。
- `load_committed_assistant`的8MiB检查改用统一原始字段长度，修复Chunked/Sealed
  inline空槽绕过预算。display window的deferred正文不能当完整snapshot；JSON在准入
  预算内用统一reader恢复原始字节（包括review与大整数），不能把省略误解为空。
  此为现有累计snapshot的正确性修复，尚未替代为公开journal/range传输。

验收：

- 实际磁盘2/8/32MiB、每次32KiB生产提交，session35063 exit0，日志
  `/tmp/nola-stream-scale-20260928.log`。Append耗时约335/1246/4977ms，SQLite缓存页
  写入2274/9217/37417，固定100次tail+range约195ms；Close及冷重开全文SHA均通过。
  精确数值、缓存/物理I/O区分、debug与固定checkpoint边界见规模验收文档。
- Store lib **261通过、1默认忽略**（规模测试已单独执行通过），remote assistant4及
  turn append sync5均通过，session57245 exit0，
  `/tmp/nola-stream-bounded-store-20260928.log`。
- Application lib **177/177**，真实stream2、turn transaction15、cancel6、run effect5
  均通过，session4944 exit0，`/tmp/nola-stream-bounded-application-20260928.log`。
  本批限定文件diff检查通过；未运行Mac/CLI实机或网络真实模型验收。

接下来必须处理：`transcript_range.rs`仍要求整个消息completed，可信internal resume
中旧已完成Part的UI全文入口可能被拒绝；需要端口级window→range回归，不能直接去掉
完成态栅栏。远端协议接收方按序列化包计算的8MiB上限与本轮原字段预算也需统一。
全Store配额、Core checkpoint增量恢复合同、各客户端公开journal消费者均仍未完成。

### P5 原文资格、审核披露与消费式 checkpoint（2026-10-01）

本批继续闭合真实生产读取边界，而非只增加展示侧条件：

- `DeferredTextPart`现在必须同时携带`is_final`和`can_read_original`。前者是Part生命周期，
  后者由Store与实际range入口共用准入函数计算，不能从消息或预览状态猜测。
  真实失败/取消消息可以已经final却不可读；可信内部续接的旧完成正文仍可读。
  Rust与Mac都拒绝缺字段、错误类型、未知字段及未完成却可读的组合。
- 原JSON的review披露策略接入普通全文、窗口资格、历史text/markdown、canonical
  preview及搜索。缺省/null/approved可见；pending/rejected/interrupted及重复、未知、
  错类型或无法取得的review均隐藏。原始字节仍完整保存，不把候选当普通回复。
  无Core checkpoint的导入历史和冷重开也按相同规则处理。
- Inline JSON控制字段由单条SQLite row借用原值分块，Sealed使用有界range；两遍
  visitor仍扫描整个JSON。此为有界内存与披露正确性改进，不是常数时间控制metadata。
- 公开journal固定snapshot/cursor的读取和释放允许可信历史Run归属；源端prepare/
  publish仍只接受当前Run。真实内部续接、冷重开、旧写权拒绝、foreign/remote/epoch
  伪造及缺失任一可信记录均有回归。公开journal生产发布仍未接到Application reducer。
- Core新增`into_checkpoint_projection(self)`，借用API克隆一次后转发，restore和
  Application durable边界消费已有owned state。删除仅做身份复制的Host journal helper。
  活动、压缩、完成、取消、失败及终态仍有pending request均验证借用/消费字节一致，
  transcript、投影、冻结context和实际ModelStream请求正文地址保持不变。
  这只消除该所有权边界的额外复制，不解决每delta回滚、累计事件或全文序列化。

新合同见[原文读取](../../软件架构设计/模块/04-存储与索引/ORIGINAL_TEXT_READER_CONTRACT.md)和
[Core checkpoint正文拆分](../../软件架构设计/模块/04-存储与索引/CORE_CHECKPOINT_CONTENT_CONTRACT.md)。后者逐项记录当前
实际成本与私有控制checkpoint/不可变正文目录目标，不能用公开展示日志代替恢复正文。

验收证据（均隔离测试数据；Rust使用`CARGO_INCREMENTAL=0`）：

- Domain **77/77**：`/tmp/nola-reader-disclosure-lib-20261001.log`。该次Store有三项
  测试夹具失败：prepared对象总配额未包含先前正文，以及错误要求无JSON历史也返回
  structured详情。修正夹具后重跑，未放宽生产策略。
- Store **272通过、1默认忽略**与StorePort **3/3**：
  `cargo test -p nola_store -p nola_store_port --lib --quiet`，session21287 exit0，
  `/tmp/nola-reader-disclosure-store-r2-20261001.log`。忽略项是先前单独通过的规模测试。
- Application **180/180** lib、六组integration **33/33**：
  `cargo test -p nola_application --lib --test canonical_stream_transition_e2e
  --test turn_transaction_e2e --test run_cancellation_e2e --test historical_window_budget_e2e
  --test historical_projection_port_e2e --test branch_transaction_e2e --quiet`，
  session14516 exit0，`/tmp/nola-reader-disclosure-application-20261001.log`。
  包括真实Host失败/取消、30KiB Unicode原字节保留、Part最终资格、续接及冷重开，
  以及实际Application窗口/详情/range/search中的审核披露。
- Core **26/26** 定向checkpoint回归：
  `cargo test -p nola_core checkpoint --lib --test agent_core_host_contract
  --test agent_turn_context_snapshot_contract --quiet`，session72100 exit0，
  `/tmp/nola-owned-checkpoint-core-20261001.log`。未运行全部Core测试。
- Mac实际重新build并test：`xcodebuild test -project Nola.xcodeproj -scheme NolaMac
  -destination 'platform=macOS,arch=arm64' -destination-timeout 10
  -only-testing:NolaMacTests/MacConversationLazyLoadingTests
  -only-testing:NolaMacTests/MacCanonicalSessionProjectionTests
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO`，session20948 exit0，
  `/tmp/nola-reader-disclosure-mac-20261001.log`，**13 XCTest +14 Swift Testing =27项**。
  结果含Rust service/CLI重新构建及Swift真实类型检查、原文窗口与键盘交互回归；
  未用旧产物的test-without-building。

本批上述定向覆盖共 **618项通过**，不代表全workspace、全平台或真实网络模型验收。
此前“消息完成态/远端预算不一致”的下一步已在本批及读取合同中处理；剩余主路径为：
公开journal全Part生产与传输、版本化全文跨页读取（同字节长度不是版本）、Core
累计事件和checkpoint去全文化、独立控制metadata、全Store物理配额/回收，以及全端
和真实模型验收。整体目标仍在进行中。

### P5 事件转移、步骤语义与阅读器请求所有者（2026-10-01续）

在上一批读取正确性之上继续改实现，并再次按当前源码验收：

- Core唯一事件封装调用者消费`Vec<AgentEvent>`，封装使用`into_iter`；移除该边界
  每次克隆累计Part正文的行为。新合同验证2MiB以上Unicode Created、80KiB delta、
  四个正文地址、所有wire字节、独立turn身份、顺序、cursor与revision；空批次不推进。
- Domain进度从有限u64有理数改为准确binary64位模式，JSON为严格decimal u64
  string、binary为u64BE。保留subnormal与signed zero，构造/JSON/binary都拒绝
  非有限或超出[0,1]值；测试包含边界及1,024个确定性有效采样，不把它们计作1,024项test。
- 新增独立`JournalStepMetadata`，真实Step adapter保留生命周期与精确标题；长标题
  走typed Title body，完成/失败/取消仅更新control metadata，不重写/prepare标题。
  equal-length换标题不能借完成事件混入。Planner与真实Store publisher桥接覆盖
  近1MiB长标题、三种终态、固定旧snapshot和冷重开；仍不是实际reducer已生产发布。
- `MacOriginalTextReadModel`拥有原文窗口的一次查询：翻页、换Part、retry、关闭
  同步取消真实token，传递到ServiceClient读取socket；generation/selection隔离迟到
  结果，预取消Task不发查询或覆盖新页。没有伪造正文版本，也没有自动重新打开。
- 再审查公开日志生产路径：除了Begin/Host/active/显式完成，还有idle checkpoint
  Task/Skill和Store recovery直写终态。已将完整publication-set、后台source proof、
  current writer与presentation owner分离写入adapter设计；不以一条无限record或
  静默跳过未知类型来接线。富Tool字段的共享映射仍需审计，标签支持不等于语义齐全。

当前源码验证（均exit0，Rust使用`CARGO_INCREMENTAL=0`）：

- session80193：`cargo test -p nola_domain -p nola_store_port -p nola_store
  -p nola_application --lib --quiet`，日志`/tmp/nola-journal-control-lib-20261001.log`。
  Application **182**、Domain **78**、Store **272通过/1默认忽略**、StorePort **3**，
  共 **535项通过**。Store规模默认忽略项未在本续批重复执行。
- session18364：`cargo test -p nola_core --lib --test agent_core_host_contract
  --test agent_turn_context_snapshot_contract --quiet`，日志
  `/tmp/nola-owned-envelope-core-20261001.log`。Core完整lib **393**、Host **115**、
  turn context **2**，共 **510项通过**；此次不是checkpoint关键词过滤。
- session1253：`cargo test -p nola_application --test canonical_stream_transition_e2e
  --test turn_transaction_e2e --test run_cancellation_e2e --test historical_window_budget_e2e
  --test historical_projection_port_e2e --test branch_transaction_e2e --quiet`，
  `/tmp/nola-journal-control-application-20261001.log`，六组 **33项通过**。
- session36153：实际`xcodebuild test -project Nola.xcodeproj -scheme NolaMac
  -destination 'platform=macOS,arch=arm64' -destination-timeout 10
  -only-testing:NolaMacTests/MacConversationLazyLoadingTests
  -only-testing:NolaMacTests/MacCanonicalSessionProjectionTests
  -only-testing:NolaMacTests/MacOriginalTextReadModelTests
  -only-testing:NolaMacTests/MacCompactionRecallEndToEndTests/testNativeRecallOfTenMillionCharactersReturnsBoundedTailEvidence
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO`，
  `/tmp/nola-owned-query-mac-20261001.log`，**18 XCTest +14 Swift Testing =32项通过**。
  千万字符历史查询约55ms、响应716字节；这是有界尾部证据查询，不是模型端到端耗时。

以上当前批定向覆盖共 **1,110项通过**。仍未做全workspace、全平台或网络真实模型
验收。公开journal完整生产/传输、版本化原文页、Core source-owned回滚与增量控制
checkpoint、完整可见字段parity、私有恢复入口及全Store配额/GC仍未完成；目标保持进行中。

同日进一步闭合checkpoint元数据边界：`runtime::checkpoint`不再调用完整snapshot
仅为取得四个小字段，直接读取session、journal、catalog。state投影仍保留原恢复语义，
没有删回滚、丢隐藏正文或跳过receipt。新增六态×17种reason与旧构造逐字节相等、
idle及cursor/revision/catalog极值回归，确认live state与终态裁剪条件不变。

- session73388 exit0：同上Core完整命令，**395+115+2=512项通过**，
  `/tmp/nola-checkpoint-metadata-core-20261001.log`。
- session24603 exit0：同上Application命令增加`--lib`，**182+33=215项通过**，
  `/tmp/nola-checkpoint-metadata-application-20261001.log`。
- session56769 exit0：同上Mac32项实际build+test命令，**18 XCTest+14 Swift Testing**
  均通过，`/tmp/nola-checkpoint-metadata-mac-20261001.log`。此结果覆盖本批最后一次
  Core源码修改，不沿用修改前的Mac可执行产物。

连同未受此Core内部改动影响的Domain78、Store272与Port3，当前定向覆盖共
**1,112项通过**，不是全workspace/全客户端/真实模型总验收。
上述是最新富公共语义实施前的通过基线，不覆盖第8.2节的新源码。
当时的逐字段审查已并入[Application公开日志合同](../../软件架构设计/模块/04-存储与索引/COMMITTED_ASSISTANT_JOURNAL_APPLICATION_ADAPTER.md)：
旧MessagePartView、富AgentPart与journal三套语义并不等价。Tool人类摘要/结构化结果/
preview要分开保留，Reasoning title/Source脱敏证据/Citation identity与section不能丢，
三种revision clock不能互相冒充。当时仅记录实际消费者及下一实施gate；后续共享合同、
Store与跨端接线的源码进展和未通过的最新回归见第8.2节，不能继续说成仅有审查。
Core事件拆分合同也补齐pending请求重复复制、config回滚缺口和全端
消费者；通过OpenAI Docs重新检索并打开官方流事件参考，设计推导与官方事实分开记载。

### P5 全字段公共语义、严格冷读与有界诊断（2026-10-01）

本批从真实生产数据合同修正，不以平台各自补字段猜测代替共享事实源：

- Domain普通Part、Application借用映射/journal planner、Store bounded semantic reader
  与跨端DTO同步富字段。Tool的summary、structured output、preview、origin、只读
  permission-display、artifacts与时间独立；Reasoning保留title；Source仅公开脱敏的
  typed MemoryUsage；Citation保留identity/location/range，Attachment保留operation。
  缺失、显式null、空字符串/集合分别保存，不将私有output路径或显示权限当作授权。
- Mac实际冷重开测试暴露共享根因：`project_stored_parts`将完整公开值一律降成
  Historical，消费者因而丢失富字段。新增`stored_public_part.rs`统一还原严格Domain
  公共值，必须有精确owner、ContentJson source、Canonical binding、Supported及
  全部Complete字段。Deferred、Core私有schema及未知/无效字段不得升格；本模块
  发现无效public合同时清空展示字段，保留身份/source证据。Source clock保留在原文
  解释中，typed public view使用本地projection revision，不冒充远端clock。
- Store公共schema拒绝未知字段，MemoryNamespace unit variant与共享Domain合法域
  一致；未批准review保留真实空正文事实但不泄漏候选，不凭空制造空markdown。
  新真实Application remote admission→commit→full/window/page→冷重开测试分别验证
  富字段/极值、Deferred大结果的精确原文及未知私有字段拒绝。旧remote identity测试
  改为同时验证完整typed view和Store原文source/digest，而不是删除原文证明。
- `event_diagnostics`独立模块先从借用Part生成字段预算，再由writer限制整条实际
  编码字节**<12,000**；Created只留一份snapshot。通用serde adapter在底层JSON编码前
  限制字符串/键/格式化结果及递归深度，保持小值和显式null。canonical结果原样
  持久化，不拿诊断预览作为唯一结果。实际AgentPart事件codec回归不是Core transition
  全链压力验收；非Part事件、自定义Serialize内部工作及Core累计状态仍有独立成本。
- Journal内容相等比较真实manifest页及transaction-local overlay，不再用假定固定
  分块判断；不同合法分块下的NoChange、同长替换、旧snapshot及冷重开均有测试。
  窄manifest查询验证精确owner/ref/geometry；Citation paired byte range与opaque URI
  合法域保持共享一致。这不等于生产publisher已接通或ToolState累计成本消除。
- iOS真实已提交窗口接入live/final draft合并，canonical同call的完成/失败/取消工具
  状态、input/output优先于迟到legacy trace/draft。恢复两个await后重验原conversation/
  generation/coordinator，提前退出释放acquired session，不放宽prepareRecovery。
  十项新rich tests使用真实v1 assistant commit/query/重开并调用实际consumer函数；
  不等于生产v2远端stream或ChatManager→Combine→ChatView异步/视觉整链验收。
  Remote非终态canonical富字段同步、缺少已提交终态的传输错误和可信冷恢复仍未闭合。

真实失败与闭合记录保留，不把阈值放宽或旧产物沿用写成成功：

| 执行 | 实际结果与修复 |
| --- | --- |
| 六crate lib r3/r4→r6 | r3新增测试使用无Ord的digest作BTreeMap键，修为HashMap；r4暴露Domain必填fixture、Store未知字段及unit namespace合法域不一致。修正真实合同/fixture后r6 **1,103通过、1默认忽略、exit0**；各crate数量见第8.2节 |
| Application全部tests r1→r3 | 首跑lib214+integration246通过/6失败；二跑lib216+integration251通过/1失败。旧fixture补真实Reasoning字段及historical turn的run_id绑定，保留budget/Deferred/未知kind断言；remote测试保留原source/digest证明。三跑**77组、216+252=468通过、0失败、exit0** |
| Mac实际build/test r1→r4 | 首两跑暴露structured JSON wrapper类型及必填fixture编译错误；三跑实际发现共享冷读降级及旧JSON字段。修复共享根因及fixture后四跑**56 XCTest+14 Swift Testing=70通过、exit0**，xcresult独立确认 |
| Android完整debug JVM | JDK21、单worker，**BUILD SUCCESSFUL；83类/311项，309通过、2跳过、0失败**，新增8项回归，实际Compose消费者编译；不含设备/JNI/release |
| iOS原生build/test r1→r5 | 七类、iPhone18Pro iOS27模拟器、单worker；首跑59通过/1失败。scalar codec修复后r2 **61/61**；统一整数codec的新literal fixture误用origin=host及live命令宣称terminal，r3/r4各63通过/1失败，Domain/权限gate按合同拒绝。仅纠正合法builtin/running fixture并补receipt/replay/无terminal proof；r5 **64通过/0失败/exit0**，独立xcresult确认，null/整数/live/cold原断言保留 |

可复查日志/入口：

- `/tmp/nola-public-semantics-libs-r6-20261001.log`：`CARGO_INCREMENTAL=0 cargo test
  --no-fail-fast -p nola_domain -p nola_store_port -p nola_store -p nola_application
  -p nola_kernel -p nola_core --lib --quiet`。
- `/tmp/nola-public-semantics-application-integration-r3-20261001.log`：
  `CARGO_INCREMENTAL=0 cargo test -p nola_application --tests --no-fail-fast --quiet`。
- `/tmp/nola-public-semantics-wire-oracle-20261001.log`：独立
  `cargo test -p nola_core --test semantic_nested_wire_oracle --quiet`，**1/1通过**。
- `/tmp/nola-public-semantics-journal-vectors-20261001.log`：独立
  `node scripts/verify_assistant_journal_vectors.mjs`再次通过identity/UTF-8/header/
  u64/typed operations/field generation/owner-bound manifest编码验证；不是生产订阅器。
- `/tmp/nola-public-semantics-mac-r4-20261001.log`：七类过滤，真实`xcodebuild test`，
  macOS arm64本机；结果包`Test-NolaMac-2026.10.01_4-08-44-+0800.xcresult`。
- `/tmp/nola-public-semantics-android-20261001.log`：
  `./gradlew :app:testDebugUnitTest --no-daemon --max-workers=1`，JDK21。
- `/tmp/nola-public-semantics-ios-20261001.log`：`scripts/run_client_tests.sh`，
  实际build/test、单worker、七类过滤；首跑59通过/1失败，不记为通过。
- `/tmp/nola-public-semantics-ios-r2-20261001.log`：同七类实际重建/重验，**61/61**；
  `/tmp/nola-public-semantics-ios-r2.tOzixf/Tests.xcresult`独立确认，未复用首跑结果包。
- `/tmp/nola-public-semantics-ios-r5-20261001.log`：新增精确整数codec后同七类实际
  重建/重验，**64/64**；`/tmp/nola-public-semantics-ios-r5.buS6Pl/Tests.xcresult`
  独立确认。v1 live receipt/replay与cold只读证据不冒充v2远端真实stream或可信恢复。

本批还审计真实CLI验收设施：`run_events`保存Part诊断，不能以其记录计数证明完整
canonical input/output。真实模型脚本helper及fifty-read PTY说明改称diagnostic，
保留原SQL/状态/权限/50个完成记录/单一summary行与provider结果断言；context-overflow
的全文证明仍是实际`content_text == prompt`，冷查询source preview只证明身份查回。
命名/注释修改不冒充真实模型重跑；PTY定向首跑**0通过/1失败/exit101**，
`/tmp/nola-public-semantics-cli-fifty-20261001.log`：连续read完成后立即逐条flush使汇总
无机会形成；test又把执行期间输入框重画的placeholder及answer preview当作终态。
修复连续read在主时间线的独立receipt槽，按execution_id拒绝旧owner，新turn/取消/
lease释放只结算已观察工具事实，不接受未审核正文；UI原位更新及一次settle有vt100
回归。保留原×50单行与50条诊断断言，额外检查真实Run完成/无pending effects，
`/tmp/nola-public-semantics-cli-fifty-r2-20261001.log` **1/1通过（90.96s）**；此耗时
不是任务完成速度已优化的证明。terminal UI34与activity11单测通过，权限交接/resize
真实PTY也已1/1通过；当时完整CLI binary与其余相邻PTY尚未执行，后续重验见下方。
生产恢复读取`run_events`
只检查未改编码的`turn_started.turn_id`，没有将Part诊断重新作为恢复/授权/回放输入。

相邻stream同锚点PTY首次还包含过时的强制pending review/Answer verified断言：fixture
只有普通SSE content/stop，没有选择review的typed control call。已改为保留主区位置、
终态无搬迁/单次正文并核验真实Run/effects及review=None；独立review安全gate未改。
重验再暴露实际短帧停顿：ModelTextPresentationGate有100ms prefix deadline，却只在
下一个provider callback检查。约200字符单帧后暂停不会发Core ModelDelta；另外
短无标点Markdown tail也有独立显示边界。消费者本地deadline检查和有界main-tail
后续已整改，不伪造provider空delta、Host事实、完成或review，不靠加长fixture绕过。

后续生产整改复用既有bounded model delivery consumer，由本地25ms idle tick检查
已接收前缀的100ms期限，不等待下一帧，也不把tick当provider delta。工具markup
完整与分片共用同一协议marker合同；deferred/tool/private reasoning仍hold。Idle前
检查同一ExecutionContext取消事实，拒绝后取消子producer、关闭队列并join；真实
consumer拒绝与用户取消分别分类，attempt barrier、Core取消及lease准入仍生效。
CLI短单行纯文本进入既有主时间线mutable tail；短/长分片删除线补closing/parity保护。
没有宣称所有旧link/强调/fence fallback或多行尾部idle调度都已迁移；进入callback后
的取消竞争仍依赖既有Core/lease gate，不承诺无锁检查后的原子取消。

实际完整CLI binary首跑`/tmp/nola-public-semantics-cli-bin-20261001.log`
**1,394通过/9失败/2忽略/exit101**，新增11项上述边界unit均通过，原review未批准不
进正文的gate也通过。九项失败分别为：四个数据源fixture没有创建真实conversation
owner；replica HTTP fixture缺当前显式hydration字段；两个Web fixture缺模型选择；
warm supervisor fixture绕过Host memory准入；fallback冷读取仍将旧SQL标量正文
当全文。已修正真实fixture/读取合同，保留原HTTP、撤权、outbox、owner/attempt与
正文精确性断言；不默认授权或默认为旧协议兼容。两项ignored均要求独立Worker
replica E2E脚本，本批没有执行，不能写成通过。

完整重跑`/tmp/nola-public-semantics-cli-bin-r2-20261001.log` **1,403通过/0失败/
2忽略/exit0（132.90s）**，九项原失败全部闭合。数据源另外明确未知owner拒绝且
不产生override；fallback在warm registry失效后以全新Store有界读取原文，精确核验
胜出part/run/attempt及跨会话隔离。以下指定PTY均在最终freeze源码实际运行：

| 真实终端目标 | 结果及日志 |
| --- | --- |
| 普通单帧正文在主区追加、完成不搬迁/不重复、无强制review | **1/1通过（5.52s）**，`/tmp/nola-public-semantics-cli-stream-r3-20261001.log`；原r1/r2失败日志保留 |
| 19字符单帧后provider暂停、无下一delta仍在主区可见 | **1/1通过（5.49s）**，`/tmp/nola-public-semantics-cli-short-paused-20261001.log`；可见时真实Run未完成，终态位置和精确保存内容不变 |
| 运行中后续输入排队并在完成后发送 | **1/1通过（5.89s）**，`/tmp/nola-public-semantics-cli-follow-up-20261001.log`；两轮真实localhost请求及canonical四条消息 |
| 窗口resize保留草稿、取消stream并接受下一命令 | **1/1通过（5.89s）**，`/tmp/nola-public-semantics-cli-resize-20261001.log` |
| 权限返回输入框不续跑、resize及取消lease交接 | **1/1通过（8.38s）**，`/tmp/nola-public-semantics-cli-permission-r2-20261001.log`；隔离fixture的目标文件保留 |
| 连续50次read在主时间线只形成一条×50汇总 | **1/1通过（91.34s）**，`/tmp/nola-public-semantics-cli-fifty-r3-20261001.log`；50条完成诊断、provider结果及Run/effects终态原断言均保留 |

fifty-read在最终freeze源码的r3补验已通过，六项指定PTY均exit0。91.34s是该隔离
50次工具工作流的实际耗时，不能把这次正确性通过写成任务完成速度已经优化。
上述localhost/SQLite/PTY是生产链路的
受控验收，不是联网真实模型或所有终端视觉/Markdown场景的完成证明。

这些测试重叠，不相加成唯一测试数。公开journal完整生产发布、传输/客户端cursor、
Core增量私有恢复及第8.4节硬门槛仍未完成；本批正确性修复不等于整体goal完成。

### 完整发布集合与目录一致性续批（2026-10-02）

本批实施完整不可扩展publication set、schema33集合凭证/成员、canonical实际字节
多帧规划及借用owner映射。首轮编译发现新fixture误用cursor字段，后续lib首跑又
发现inline预算、返回head排序与真实source fixture问题；修正测试事实，不改变生产
约束。六crate lib最终r3为1,119通过/0失败/1默认规模忽略；完整Application/Store/
Domain首跑仅旧Tool draft fixture缺必需字段失败，补真实title/origin/artifacts及
span断言后，完整r2为122组、1,037通过/0失败/1默认忽略。显式私有Text规模
2/8/32MiB、Close/冷重开/全文SHA另1项通过；Core独立wire oracle及JS vectors均通过。
共享层新增16项，首失败/最终日志分别保留，见8.5和规模专项。

CLI完整首跑1,401通过/2失败/2忽略，两个失败均为跨读取Skill目录变化。整改实际
Web bootstrap/resolver的local+host共源及目录内部flags一次capture，保留当前执行
检查、Core deny、原schema和deferred断言；新增5项回归。Web5、availability3、
Skill schema3定向均通过；完整默认并行r2为1,408通过/0失败/2忽略（78.38s），
没有以test-threads=1掩盖竞争。两个CLI产品binary也已完整重建exit0（1m05s），
详见8.6，不把子进程1项或重叠suite累加为唯一新增数。

本批没有重新运行原生/Web、PTY或联网真实模型。实际审查确认两项未闭合根因：
旧inline分页逐页全JSON披露扫描；Skill实际root仍process-global且Host目录捕获
早于本Run配置绑定。后续按8.6做Runtime所有权迁移，并按8.7补typed变化全集、
窄owner/order/successor端口与idle/recovery真实source，保持hot batching/UoW目的
差异。producer/transport仍关闭，不能将这批回归通过称为完整架构整改完成。
