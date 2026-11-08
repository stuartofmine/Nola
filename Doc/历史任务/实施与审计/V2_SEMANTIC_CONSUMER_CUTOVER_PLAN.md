> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 历史 Part 语义消费者切换：窗口与模型上下文共同接线

2026-09-08。下文保留切换前审计与验收目标，不再代表当前生产解码。
窗口、新轮次上下文与 idle revert 已共同消费 HistoricalPartProjectionRepository；
实际实现、测试终态和未完成项见 [当前架构最新核对](V2_CURRENT_ARCHITECTURE_2026-09-08.md#最新核对历史窗口模型上下文与回退已共同接线)。
接口实际采用逐 Part 读取与 Application 共享预算，并非下文建议的 batch 签名；
完整逐 kind 类型、跨 Part 引用、统一 omission counts 与 token-range 交互仍未完成。

## 真实消费者及现存分歧

`core/nola-application/src/conversation_projection.rs::project_stored_parts` 当前依次尝试完整 `content_json -> MessagePartView`、完整 `content_json -> AgentPart`。前者只检查 part_id/kind，后者直接交 `project_core_part`；识别失败且canonical kind=text则显示content_text，其他kind令整个投影失败。没有消费 owned semantic draft。

调用入口有三类：

- `load_remote_projection_subset` 对选中的 StoredMessage 调该函数；不能因 remote session 有权威运行投影而跳过历史 Part 的接收端 owner 检查。
- `materialize_conversation_projection` 对 user/system及没有Core投影的历史assistant调用它。`materialize_local_presentation`在有Core checkpoint时先抽取所选Message的本地呈现输入，没checkpoint时走 imported materializer。活跃Core Part来自Core投影，可以保留现有 `project_core_part`；历史原文分支必须走新共同契约。
- `load_conversation_projection_window_v1` 先取Store窗口，再走上述local/remote路径。page/full projection、窗口更新、tail及订阅快照还会复用这些materializer；只修window一个RPC无法防止其它读取重新进入旧decoder。

`core/nola-application/src/lib.rs::durable_conversation_items` 独立完整解码 AgentPart，直接使用嵌入 id/call_id 产生AgentItem；没有核对canonical Part owner。Tool input/structured_output进入完整Value；解析失败或Source/Citation/Artifact/Step只被忽略。一个Message若没有产生item，才把所有content_text拼接成fallback；因此部分成功的Message可悄悄丢失其他Part。

它的生产调用为：

1. `canonical_branch.rs::restore_canonical_context`，由 `lib.rs` begin-turn路径在需要恢复规范历史时调用。先选最近32条Message，预算 `(prompt_token_limit / 4).clamp(128,32768)` UTF-8 bytes，之后仍把窗口 `content_json`交给上述完整decoder。截断说明只覆盖deferred_text_parts，不能表示结构化字段缺失。
2. `lib.rs` revert用例：先完整load_conversation定位可见boundary，在空Core history时再次完整load，并用该函数rebase，再执行revert。此入口没有窗口预算，必须一起收敛，不能只修新turn。

Store `lib.rs::load_conversation_message_window`已pin snapshot并传limit/content_byte_limit至`load_conversation_projection_with_window`。但StoredPart的Option<String>无法表达JSON字段Absent、Deferred、Unsupported三种不同事实；不能继续把窗口里没带回JSON当成“不存在/不支持”。

## 已有能力可直接复用的边界

- Domain `part_semantics.rs`已有CanonicalPartOwner、SemanticSourceKey、SemanticTokenLocator、`SemanticValue<T> = Complete | Deferred`。locator只是描述；后续读取仍需canonical owner/visibility/revision/source校验。
- Store `semantic_json_projection.rs`负责原始流式语法+hash和token提取，最多64字段、输出≤32768bytes；禁止把Deferred当null或可执行截断值。
- `semantic_part_draft.rs`识别AgentPart/MessagePartView，明确WrongType/Duplicate/DeferredControl等Unsupported，且保留pending_nested_validation。
- `semantic_nested_draft.rs`现只覆盖Origin/Permission/AffectedResources；不能把pending_nested_validation非空当完成typed投影，也不能用serde完整解码一个大nested token补洞。其数组结果只有全部验证后才提供resources，不能把prefix当完整数组。
- `part_semantic_source.rs::inspect_part_semantic_draft`绑定真实可见owner/source和canonical run，再调用Kernel身份判断。root正在补localbranch绑定判定与Store adapter，应消费其结果；不可在Application再造一套身份规则。

## 最小完整实现切面

新增一个**立即被窗口与context共同调用**的StorePort `HistoricalPartProjectionRepository`：

`project_historical_parts(selection, expected_conversation_revision, budget) -> HistoricalMessageProjectionBatch`

selection包含canonical Message/Part身份或分页边界，不包含客户端声明的owner替代、原JSON副本或授权结果。Store在同snapshot选visible记录、读取原始body source、识别schema、nested验证、调用Kernel绑定判定，返回Domain typed结果。批次预算必须有总encoded bytes、scalar bytes、Part数与nested项数，不可每Part各给32KiB后无限累加。字节只从原source reader读取，禁止重新取整个content_json。

共同Domain结果建议为 `HistoricalPartProjection { owner, canonical_revision, source, binding, content }`：

- `binding`只接受Store调用Kernel得出的Canonical/LocalBranchMapped/Unsupported；保留结构化reason code。外部输入不能指定。
- `content`按真实历史schema适配为typed Text/Reasoning/Tool/Source/Citation/Attachment/Task/Compaction，以及Unsupported。大字段使用已有SemanticValue，option外层表示原字段不存在，Complete(null)只表示合法现存null。
- `Unsupported { code, canonical_kind, original_json: Option<SemanticTokenLocator> }`不伪装成text，不暴露一个能直接被当作ToolCall执行的半解析对象。权限字段、资源数组未完整验证不能产生活跃权限/资源动作。
- source token locator是原JSON字节区间；展示解码与原文读取必须区分，不得把JSON string含引号/escape的raw bytes冒充用户可见字符串。

Application新增一个共同adapter模块 `historical_part_projection.rs`，接收以上typed结果生成presentation/context；删除两个生产函数中的serde_json兜底解码，而不是让新adapter与旧decoder并存。小字段Complete可映射现有MessagePartView；Deferred或Unsupported必须有显式可显示状态。

### 窗口 DTO 与五端

Domain `projection.rs::MessagePartView`增加一个明确的历史语义变体（例如Historical），承载canonical part_id/revision、typed kind、绑定结果、typed字段complete/deferred状态、Unsupported reason与可读取locator。既有普通Text/Tool等及活跃事件不改变含义。若选择给每个旧variant逐字段套SemanticValue，则需要同步改全部live writer/reducer，改动更大；不建议这轮采用。

`ConversationProjectionWindowV1`保留text excerpt信息，并新增统一semantic selection completeness/omitted counts；不要把所有semantic缺失藏在deferred_text_parts。批次预算不足时允许返回完整控制字段+Deferred数据；控制字段本身不足则显式Unsupported/BudgetExceeded。Message级不要静默丢Part。

必须同步：

| 消费端 | 实际文件/路径 | 最少新字段行为 |
| --- | --- | --- |
| macOS | `NolaMac/Services/MacApplicationPort.swift`的projection Part DTO；`MacConversationProjectionClient.swift`及canonical repository呈现转换 | 历史typed kind、Complete/Deferred、reason、locator；不能默认decode成text或提供执行按钮 |
| iOS | `Nola/Models/IOSConversationProjectionV1.swift`、`Nola/UI/Chat/IOSConversationProjectionPresenter.swift` | 同上；未知历史变体需明确unsupported显示 |
| Android | `android/.../domain/projection/ConversationProjectionV1.kt`及其presenter | 同上；不得用原JSON在Kotlin二次识别 |
| Web | `web/app/static/js/agent-trace-runtime.js`的Part转换；CLI Web projection routes | 识别Historical，展示历史tool/source及deferred状态；locator经Core请求，不能拿source URI直接代替授权读取 |
| CLI | `cli/src/main_parts/bootstrap.rs`规范projection消费、`cli/src/store/web.rs`window适配 | 文本renderer给出历史语义/unsupported与原文读取提示；不能仅保留content_text使结构化Part消失 |

Rust Kernel projection reducer与`core/nola-mobile-bindings/src/projection.rs` FFI也消费MessagePartView：新variant需支持identity/revision提取、snapshot恢复及replace事件；历史semantic节点不能被append_text_preview误当live Text。不是“Swift加个字段”即可闭环。

### Context接线与预算

`canonical_branch.rs::restore_canonical_context`在同选择快照调用共同typed repository，以现有最多32条/32768bytes基础预算，向共同adapter显式申请Context消费；不能从展示preview再反推原Tool输入。

- Canonical/经验证LocalBranchMapped且必要字段Complete：可构造历史AgentItem。ToolCall和ToolResult以同一个映射表维护call_id，不能仅映射Part ID而保留冲突call_id。
- 必要字段Deferred或Unsupported：保留一条有界历史证据说明与locator，不生成不完整的ToolCall，不伪造null/{}参数，不把截断output当完整结果。若一个pair无法完整表达，按pair降级，避免孤立ToolResult。
- Source/Citation/Artifact/Task这类目前被忽略的Part，必须至少贡献有界typed历史说明；每Part消费状态可观测。
- budget计算包含生成的AgentItem文本、JSON、定位说明与元数据，不只原content_text。完整/省略计数写入现有selection notice；用于模型的说明与UI显示均来自共同typed状态。
- revert路径分开“全局boundary定位”和“有界context选择”：Store查询可见Message身份/ordinal定位及生成hide intent，不能为定位边界load全部正文。空Core rebase改走同bounded context函数；若revert要求Core知道未选中的旧boundary，增加明确canonical boundary映射/重建提交契约，不能传一个已不存在的item ID或悄悄回退full load。

## LocalBranch身份与Archive/Replica的明确边界

current target owner是窗口/context所有输出身份。原JSON id/message_id/turn_id由root Kernel判定与localbranch proof绑定；root/immediate持久group可在source删除后验证。仅字符串相同、root_source非空或matching SHA都不能单独授权映射。

需要一次批次级映射：`(validated source turn, source message, source part, source call) -> target canonical message/part + deterministic context turn/call`。只有typed control字段参与，不修改原JSON、不替换嵌在任意字符串中的ID、不创建原run记录。Citation origin_tool_call_id等交叉引用必须引用同映射域；找不到对应call则明确UnsupportedReference，不自行生造可调用identity。

Archive/Replica provenance的**派生身份重映射**明确Unsupported（即使claim自称本地）。直接canonical匹配且schema/nested均合法的记录可按普通canonical历史规则评估；不能把全部Archive/Replica原文都禁用。需要映射但不具LocalBranch证明时保留原文、typed可显示的非授权信息及reason `untrusted_derivation_origin`，不生成可用于上下文ToolCall/权限/执行的结构。实际枚举名服从root本轮Kernel，不另造不兼容判断。

## 不能省略的真实验收

1. 同一小fixture走window与新turn context，两端对Text/Reasoning/Tool/Source/Citation/Artifact/Task/Compaction的kind、owner、call映射与完整性判断一致；Message含多个Part时不漏掉Unsupported成员。
2. 真Service 10 MB content_json多字节跨chunk，window/meta+tokenrange帧有界，模型context budget包含生成说明；不load整JSON、不写preview回Store。
3. local branch两代、删source重开，window仍显示新canonical Part ID，context tool-call/result/citation互相指向正确映射，无碰撞、无旧run恢复；原JSON完全不变。
4. Archive与Replica同字节伪造root/local标记、错误target、无proof、重复JSON control key、nested invalid/Deferred：都不能提升绑定；window显式Unsupported，context无可执行半结构。
5. stale revision、隐藏owner、source切换、原字段同长篡改、token locator移植拒绝；读取不会写receipt或改变history。
6. idle revert无checkpoint、boundary在context窗口之外、大历史：有界且重试稳定，目录/消息可见性与Core rebase一致。禁止仅测restore_canonical_context而遗漏revert老路径。
7. 五端真实decoder/presenter与native Rust reducer snapshot恢复；未知状态不崩、不丢Part，不对历史权限/资源显示可执行控件。
8. 验收扫描两个旧入口不再出现 `serde_json::from_str::<AgentPart/MessagePartView>(part.content_json)`，真实窗口、新turn、revert都调用共同typed链。否则新增seam不算完成。
