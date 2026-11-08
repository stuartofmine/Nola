> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# LLM 场景、配置与模块交互审计

日期：2026-10-15。状态：首批模块边界整改已实施，等待主代理统一编译测试；不是全场景改造完成声明。

## 本批实施边界与接口

- `model_profile_resolution.rs` 从 Web 私有实现提取 Host 共用 profile → route/能力/参数解析器，Web 与委托使用同一实现；`ResolvedLlmSettings` Debug 不输出配置/密钥。
- `delegated_invocation.rs` 输入已组合父请求、owner、agent profile、store，输出无密钥的 `DelegatedInvocation`。任务 metadata 保存它；恢复以其 profile/prompt/tools/runtime 为准，不重新读取当前 agent registry。显式模型配置由裸字符串改为 `RunGenerationIntent`（profile ID/revision/tier/overrides），旧裸字符串拒绝，不保留兼容支路。
- 凭据引用匹配 endpoint identity，显式 profile 从 Host credential vault 读取；继承配置只能从恢复调用者中相同路由取凭据，路由不匹配失败，不借别的 key。URL query/userinfo不能写入快照。恢复可加入新限制，不导入新授权。
- Swift `LLMSummaryInvocation` 捕获 owner/operation/scenario/provider revision；`ProcessingAgent` 不再在 ModelService 重试之外叠加一次严格提示重试。普通 `ModelService.generateSummary` 每次调用固定 provider，退避取消不再被吞掉。
- 所有改动只针对配置/调用/恢复；不改UI、会话草稿，也不生成第二份plan/goal/context状态。新测试已补但本子代理未执行。

未完成边界：全场景统一 typed ScenarioRegistry、跨端可配置场景策略UI、后台摘要的持久运行账本与崩溃续跑还未实现。`processing-agent` 是后台组件owner，不冒充会话owner；operation独立。Android会话generationOverride目前只覆盖context/output，完整采样会话编辑仍需后续接入。此批不宣称这些目标已完成。

当前 agent 配置格式（省略 model 则继承已冻结父配置；不是传 provider 的裸 model 名）：

```json
{"agents":{"review":{"prompt":"Review evidence and report findings.","model":{"profile_id":"configured-profile-id","expected_revision":3,"tier":"balanced","overrides":{"temperature":0.2,"max_output_tokens":8192}},"tools":["read_file"],"max_turns":6}}}
```

本批已完成 Rust formatter parser 与 Swift frontend parse 静态语法检查；未运行类型检查/构建/测试。新 Rust 单测覆盖无密钥roundtrip、同端点凭据与跨owner拒绝、配置漂移隔离、显式profile完整route/revision/参数解析；Swift单测覆盖provider更换后辅助调用仍使用原provider且只调用一次。仍需主代理统一跑这些测试及既有委托恢复、Web profile解析、ProcessingAgent测试。旧任务缺少冻结契约会明确失败，不用当前全局配置猜测恢复。

与 [会话所有权审计](SESSION_OWNERSHIP_AUDIT_20261015.md) 配套：配置问题不能只在输入框修补；客户端、会话、运行、子任务和物理模型请求必须有明确身份与不可变边界。

## 1. 结论与证据范围

不是从零重写：已有 Core Turn 冻结、Host 输出准入、模型调用账本、上下文 checkpoint、plan revision、goal 证据与子任务回执。缺口在于这些机制尚未构成**跨所有 LLM 场景的统一配置解析契约**。

以下「确认」指已阅读的具体实现，不代表运行复现；「风险」是从代码边界推导，需随后测试；「设计」尚待实施。本次没有用文件名存在代替行为验证，也没有认定所有旧入口都在当前产品路径被调用。

### 已确认的关键缺口

1. 子代理配置 `RuntimeAgent` 只有 `model: Option<String>`，没有完整 profile 身份/revision、场景采样、reasoning 或预算覆盖。`cli/src/runtime_config.rs:140`、`agent_registry.rs:8`。在 `task_runtime_parts/supervision.rs:3519` 先复制父配置，`:3550` 仅换 model 字符串。**确认配置表达能力不足**；跨 endpoint 模型被错误地套用父协议/能力是风险，尚未声称某次请求已发生。
2. 子任务恢复重建时继续读取 registry 中的 profile，并拼接 `profile.prompt`（同文件 `:3595`）。任务身份与持久会话已存在，但 profile/model/prompt 的完整独立冻结契约没有在此恢复边界体现。需要验证编辑 agent profile 后恢复是否改变原任务；不能把「复制当前 config」称为「恢复原始配置」。
3. `Nola/Services/ProcessingAgent.swift:122` 调用 `container.llmClient.generateSummary`，并在空结果后自行再次调用（`:132`）；`Nola/LLM/ModelService.swift:67` 内部又有 retry loop，直接使用实例当前 provider。此接口没有 conversation/run/scenario/config revision 参数。**确认独立辅助摘要入口与主 Run 准入契约不一致**，存在叠加重试与配置归属不透明；是否与当前聊天共享同一实例须继续沿容器装配确认，不能仅据此断言串会话。
4. 配置类型分散为 `RuntimeConfig`、`HostInvocationRequest`、`ModelInvocationSettings`、`RunGenerationIntent`、Core session/turn configuration。分层本身合理，但未见一个涵盖 chat/compaction/subagent/decision/plan 的统一 typed purpose + 字段来源账本。`RunGenerationIntent` 已有 profile/revision 与稀疏参数，适合作为收口基础，不应再造一套同义 JSON。
5. `core/src/decision.rs` 是将已有 ModelOutput 转成通知候选的确定性函数，**不是独立 LLM 决策服务**。不要因名字为 decision 就配置另一个模型。真正需要 LLM 的轻量任务应显式登记；纯规则归约继续零模型调用。

## 2. 现有链路与应保留的能力

| 场景 | 当前链路 / 代码证据 | 已有边界 | 尚需收口 |
|---|---|---|---|
| 主对话 | `model_selection.rs::resolve_chat_model` → `run_generation_intent.rs` / Host 准入 → `agent/runtime/model_planner.rs::request_model` → `agent_host_driver_parts/orchestration_execution.rs` → provider | 显式会话选择不可用不静默换模型；意图不能带 endpoint/secrets/capabilities；Turn 持有 configuration 副本 | 所有入口共用解析结果及字段 provenance；客户端不重复选协议或预算 |
| 压缩 | `agent/runtime/compaction_driver.rs` / Core compaction request → Host `orchestration_execution.rs` 的压缩执行 → `compaction_summary.rs` → Core completion | 热分叉 witness、configuration identity、原子 snapshot 文件；取消保留原上下文；工具调用摘要拒绝；截断续写、冷分段与有界重写 | 作为 maintenance 子操作归属原 Run；统一成本/取消/策略记录；不得通用 resolver 改写热分叉前缀 |
| 子代理 | `spawn_agent` → `task_runtime_parts/execution.rs:2584` → durable task / child session → supervision →独立 Host 执行 | call_id 幂等，child session，权限限制，工具 allowlist，恢复与终态回执；native MLX 路由不支持独立通道时显式拒绝 | 完整 ChildRunSnapshot、profile revision 与继承清单；不只换 model 字符串 |
| 完成审查 / 收尾 | Core `model_planner.rs` 的 `CompletionReviewPacket`、`StructuredAudit`、final synthesis → 同一 Host model effect | 审查 packet 标明原始用户请求、候选、证据及权限；模型可选择 request_completion_review；Host 根据 plan 禁止工具/推理 | 用 typed purpose 记录审查/收尾；预算及选择可配置，但不能把可选审查改成自动必需 |
| plan / goal | `conversation_plan.rs`、`conversation_plan_store.rs`、`agent/goal.rs`、`runtime/goal_work.rs`；模型通过工具更新 | plan revision/frozen_revision、证据/工作事务引用；goal从无损 checkpoint 而非压缩摘要重建证据 | plan 是持久业务状态，不默认等于独立 LLM；规划调用若独立发起，须共享准入契约 |
| 辅助摘要 / 小型决策 | Swift ProcessingAgent → LLMClient/ModelService；Core decision确定性消费输出 | 有本地错误处理/遥测 | 归属键、typed purpose、独立限额、统一 attempt 账本；须先确认实际产品入口 |

Core `agent/turn.rs:3455` 的 `AgentTurnContext.configuration` 与 `agent/runtime.rs:432` 的捕获，证明已存在冻结 Turn，不应描述成完全没有快照。Host `orchestration_execution.rs:1117` 复制 llm 后应用 model/能力/输出/cache 约束，证明「冻结基础配置」与「每步调用投影」应是不同对象。

## 3. 唯一所有权与配置优先级

持久化 owner 沿用四层：Application defaults / Conversation aggregate / Run immutable snapshot / Conversation view state。**ScenarioPolicy 是解析政策，不是第五份会话真相。**

身份：`ConversationKey=(authority/store, account, conversationID)`；`RunKey=(ConversationKey, runID)`；`OperationKey=(RunKey, operationID, generation)`。子代理另有自己的 ConversationKey/RunKey，保留 parent linkage。不同 host 的相同 conversationID 不能撞 owner。

统一解析流水线：

`GlobalDefaults → ConversationOverrides → ScenarioPolicy → FrozenRunSnapshot → FrozenStepRequest → ProviderAttempt`

这是处理顺序，不意味着靠后的场景政策可任意覆盖用户参数：

- 全局 profile/catalog 提供默认值和真实能力；会话稀疏覆盖优先。
- 场景 default 只填未指定项；显式的会话场景覆盖高于全局场景默认。字段带来源，不通过值等于默认值推断是否显式。
- `inherit`、显式清空（provider default）、具体值是三种状态，沿用现有 generation override 语义；不可用 `null` 吞掉验证错误。
- 场景硬约束只能收窄（例如审查无工具），要返回明确 effective/reason；无法满足的显式参数在准入前报错，不静默换模型或随意调低。
- 逻辑预算与物理模型能力分离：用户输出偏好不等于无限物理输出，物理限制必须有能力证据；不重新加入任意 4096 摘要上限。
- Run 开始后改全局/模型中心/会话设置只影响下个 Run。控制面权限撤销可立即收窄，作为独立 policy revision 事件，不篡改原快照。
- 热压缩直接继承最近实际请求 witness；不重新按当前默认值解析、不换模型/删 tools/改 system 来冒充缓存复用。冷路径是另一个显式 operation，仍受当前 Run 约束。

### 字段决策归属

| 字段 | 唯一决定边界 | 持久化 / 禁止事项 |
|---|---|---|
| model/profile/provider/endpoint/protocol | 接收执行的 Host catalog + Application resolver | 保存 profile ID/revision、route identity；不得让远端客户端提供任意可信路由 |
| credential | Host credential resolver | 快照只保存 credential ref/安全 identity，不写密钥；发请求前短暂解析 |
| sampling/reasoning/tier | 统一 generation resolver，基于最终 route capability | 保存 requested/effective/source；tier是产品意图，不是 provider wire参数 |
| 输出/上下文预算 | Core预算/Host能力准入 | 保存 physical cap、logical policy、reserve与准入来源；不由视图估算裁剪 |
| tools | Host catalog声明，Core Step投影编译，权限运行时评估 | 保存catalog/projection revision/digest；工具可见不等于获权 |
| system与历史 | Prompt/Context compiler从冻结政策和canonical state构造 | 稳定有序组件、source refs、digest；网页/工具/摘要是数据，不升级为system权限 |
| cache identity | 实际协议编码后的PreparedModelRequest | 区分模型/路由/system/tools/messages/reasoning；不能拿工具集合digest当实际KV命中证明 |
| retry/fallback/cancel | Core operation政策 + Host单次attempt执行 | attempt有独立ID，已经输出/副作用未知时不得透明重放 |

建议统一 `ResolvedInvocation` 接口承载这些信息，而不是客户端公开全部字段。优先组合现有 `RunGenerationIntent`、`HostModelRouteBinding`、`HostOutputAdmission` 和 Turn snapshot，删除重叠解析；不添加旧格式兼容支路。

## 4. 模块输入输出契约（目标）

1. **ConfigurationResolver**：输入 owner、expected conversation/catalog revision、typed scenario、稀疏 intent；输出可序列化无秘密快照及每字段来源、诊断。纯解析与凭据读取分离。CAS冲突不先发模型请求。
2. **ScenarioRegistry**：登记 Chat / Compaction / DelegatedTask / StructuredDecision / Planning / CompletionReview / FinalSynthesis。每类指定继承规则、允许覆盖项、输出schema、工具政策、上下文政策、重试与资源限额。场景由明确调用者/状态机选定，禁止自然语言关键词路由。
3. **ContextManager**：输入 owner、canonical journal watermark、summary lineage、goal/plan refs、允许资源；输出不可变 ContextProjection。已压缩摘要+未压缩尾部来自一个修订；原文召回用有权限的原文索引，不能凭summary重新创造历史。
4. **InvocationCompiler**：输入快照+ContextProjection+Step tool projection；输出实际 PreparedModelRequest及前缀/预算见证。不读取当前UI选择，不执行模型决策。
5. **ModelExecutor**：只执行已准入请求，输出 ordered deltas/usage/terminal receipt。回调携带OperationKey，不更新“当前选中会话”。细节日志不得包含credential。
6. **TaskCoordinator**：输入授权的SpawnRequest、父快照、明确上下文交接包；输出child identity、冻结child snapshot和task receipt。取消/终态/消息有因果ID和幂等序号。
7. **PlanGoalStore**：输入expected revision与命令/证据引用；输出committed revision。summary只是投影，不拥有完成状态；压缩后从存储恢复约束、未完成项、用户细化、任务状态与已验证证据。

## 5. 父子、规划、轻量决策的具体规则

- 默认子代理继承父亲**已冻结**route/generation/policy，不读取当前全局默认。显式选择另一个agent profile时解析完整profile身份，而非只替换model名称；父请求cache不能冒充子请求cache。
- ChildContextPacket至少包含父owner/run、用户委托、约束、可读证据refs、goal/plan revision、交付要求与权限上界。不要无差别复制全部父工具输出；只复制明确选定范围，缺失内容可通过授权引用补读。
- child结果是证据/建议，不是父任务完成或授权。结果记录task/attempt、状态、artifact refs、未验证事项；父应用交付回执后才更新状态。重复交付不重复执行。
- 子权限是父授权与profile限制的交集；子审批决定归属于具体child operation。父allow-all不意味着操作系统或远端能力存在；缺失能力与等待用户授权必须分开。
- plan是业务记录，规划可以由主模型直接更新，不强制加一次专用模型调用。只有显式Planning operation才应用规划策略，不自动创建长期goal或审查。
- 轻量决策必须是有限输入、明确schema的独立operation；不将其内容直接加入用户正文，不继承完整聊天工具catalog。禁用工具与无副作用是默认，若确需工具须声明和准入。小模型仅是显式配置选项，不自动便宜化压缩。
- ProcessingAgent之类后台工作有自己owner/scenario/budget；不能为了复用ModelService而临时切换当前聊天provider。是否在前台可见由事件展示政策决定，不混用聊天streaming布尔值。

## 6. 失败、恢复、压缩与信息交互

- 区分 transport error、协议/schema错误、权限等待、能力缺失、取消、预算耗尽、执行结果未知。显示层不能把它们都变成“重试”。
- 每次物理attempt只有一个重试owner；Swift外层空摘要补偿不能叠加不透明的provider重试。重试额度、退避和已产生输出须计入同一operation账本。
- 压缩完成采用source watermark/lineage CAS：失败或取消保持原summary与tail；晚到结果不得覆盖更新后的上下文。热分叉失败可显式冷退避，但记录原因和额外成本。
- child/plan/goal状态独立持久，摘要仅引用。冷恢复先恢复快照及引用revision，再决定是否可续执行；凭据不可读时明确等待配置修复，不静默用新全局模型接管旧任务。
- 显式变更模型重新准入并生成successor run/operation lineage，不偷偷修改旧请求身份。工具结果已知/未知决定是否可安全重试，不由模型一句“失败了”决定。
- 取消传递到具体operation及其owned children；detach需显式policy。会话切换只改变订阅，不取消其他会话，也不占用新会话输入框。

## 7. 直接相关的参考实现

- OpenCode本地 `参考资料/opencode-dev/packages/opencode/src/session/llm.ts`：`StreamInput`显式传model/agent/session/tools，`small`有独立options，agent/model/variant合成位于同一调用边界。借鉴显式调用契约与统一合成；不要照抄可变插件hook为第二配置真相，也不改变Nola会话覆盖优先级。
- pi durable本地 `参考资料/pi/packages/durable/src/harness/compaction.ts:145`：把model ref、thinking、options、tail与firstKept写入任务checkpoint，恢复时从固定tail构造范围。借鉴持久阶段与不可变范围；该实现`:163`起更换system且`cacheRetention:none`，**不符合我们热分叉目标**，不照搬缓存策略。
- Nola自身已有 `../软件架构设计/模块/09-上下文与压缩/HARNESS_CONTEXT_PROJECTION.md`、`../软件架构设计/模块/09-上下文与压缩/CONTEXT_AND_ACCOUNTING.md`：有序tool编译、冻结证据、真实物理请求witness优于另建摘要工具系统。此次整改必须连接现有账本，不能重建一套平行上下文历史。

## 8. 实施顺序与验收

### 2026-10-15 会话配置闭环实施

- 共享 Store 的 create/updateSettings/updateProfile/patchConfiguration 在同一事务初始化首次选择 configured 模型的参数；重选保留已冻结参数。客户端只提交 modelSelectionID 和 expectedRevision，不重复默认参数策略。Web 新建解析默认选择一次，创建响应读回 canonical 投影；旧会话提交不再借用模型中心当前选择。
- CLI `conversation_invocation` 负责进程入口恢复与提交前配置交接。可直接读取其他客户端的 canonical configured 模型及 modelOverrides，复用公共 profile/session override resolver；不存在会话配置才明确失败，不回退当前全局模型。CLI 独立未注册路由保存无凭据 cliInvocation，路由凭据仅在 dispatch 前按原 identity 重绑。
- CLI 配置基线保存“上次观察的持久配置”和“本地解析值”：无本地修改则提交前重新读取 canonical；有本地修改且另一客户端已变更则拒绝覆盖。公共配置优先于 CLI 专属运行参数，累计使用量继续读取原请求账本。
- CLI `/new` 重置到进程启动应用默认，不复制上一会话配置；输入循环在无本地修改时刷新 canonical 配置，避免执行已刷新但界面编辑基线仍停在旧值。
- 尚未完整覆盖：local/connected-host 模型的跨客户端 CLI 执行适配、CLI 的非标准 agent/工具目录配置跨端编辑，以及真实多进程竞争与模型恢复测试。不得把这些边界宣称全端验收完成。
- 已进入统一验证：Rust 编译检查及以下回归通过；真实模型与各端交互验收不能用这些 fixture 代替。

### 统一验证记录

以下命令在仓库根目录执行；涉及大型 runtime 测试的命令设置 `RUST_MIN_STACK=16777216`。默认测试线程栈运行 task_runtime 时曾在 cold_permission_resume 用例 stack overflow；增大测试线程栈后 62 项全部通过。测试大二进制仍有 Apple linker 的 `__eh_frame > 16MB` 警告，不是源码未使用变量/导入警告。

| 命令（cargo test 后参数） | 通过数量 |
| --- | ---: |
| `-p nola_cli --lib delegated_invocation -- --test-threads=1` | 5 |
| `-p nola_cli --lib conversation_invocation -- --test-threads=1` | 3 |
| `-p nola_cli --lib creation_configuration_tests -- --test-threads=1` | 1 |
| `-p nola_cli --lib session_override_translation_tests -- --test-threads=1` | 4 |
| `-p nola_cli --lib task_runtime:: -- --test-threads=1` | 62 |
| `-p nola_store --lib configuration_initialization_tests -- --test-threads=1` | 1 |
| `-p nola_application --test canonical_stream_transition_e2e --test trace_port_receipt_e2e -- --test-threads=1` | 9 |
| `-p nola_application --lib execution_trace_journal -- --test-threads=1` | 2 |
| `-p nola_application --lib conversation -- --test-threads=1` | 39 |
| `-p nola_cli --test conversation_configuration_web_e2e -- --test-threads=1` | 3 |

真实临时数据库用例验证 A/B 会话独立参数、全局变更不改恢复值、共享 configured 模型更新在 CLI 下次提交生效、陈旧编辑冲突。Web E2E 使用隔离 HTTP 模型 fixture，验证实际请求参数和 durable traces，不声称是生产模型验收。测试发现并修复 PATCH 回执遗漏 `modelSelectionID`；显式选择自动模型改为当次解析并冻结应用默认，而非保留会漂移的 null 链接。新轨迹回滚测试 fixture 使用构造闭包而非对不实现 Clone 的 ApplicationCommand 调用 clone。

先批量修改、静态审计，再统一测试；这里仅列目标，不宣称已有收益。

1. **P0类型/所有权**：统一Scenario与ResolvedInvocation，字段来源、owner/revision、明确inherit/clear；文档和API同时改，不保留旧兼容解析。
2. **P0入口收口**：主对话已有Host准入保持；子代理改完整profile并持久快照；追踪ProcessingAgent装配，将仍有效后台入口接入统一调用，不删除未知在用入口。
3. **P1交接与恢复**：ChildContextPacket、parent linkage、plan/goal revision、source watermark；恢复与取消不读取新UI状态。
4. **P1压缩与轻量决策**：压缩继承实际witness；小型决策独立schema/预算/遥测，规划不强制新增请求。
5. **P1端侧呈现**：UI显示本会话effective配置、来源和下一Run生效语义；跨端只发送intent并订阅owner事件。

验收不变量：

- 所有已登记模型入口100%携带owner/scenario/config revision；缺少契约不得发请求。
- 参数矩阵覆盖inherit/clear/explicit、0/false、跨provider、能力冲突、profile删除/CAS冲突；显式会话覆盖不被全局更新改写。
- 同时A/B运行且切换UI、改默认/agent profile、压缩、child恢复：已冻结请求参数及owner不变，晚回调串会话数为0。
- 恢复同一child/operation快照hash一致；重复终态/消息/取消不重复副作用；权限不可扩大。
- 热分叉工具/system/原历史/reasoning与原实际请求一致，只有合法尾部变化；缓存不可用单独记录，不把unknown当0%。
- 每scenario分别观测排队/准备/首token/完成p50/p95、物理attempt、输入/输出/cache read/write、压缩成本及任务完成率。先测基线再定延迟目标，不以减少token数代替任务成功。
- 使用隔离fixture验证上述契约后，真实本地192端点跑多轮对话、child、压缩、取消恢复；记录非特化任务结果。当前文档审计未运行这组验收。
