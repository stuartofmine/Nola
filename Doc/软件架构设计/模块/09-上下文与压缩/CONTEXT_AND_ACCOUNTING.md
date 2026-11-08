> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 模型上下文、热快照与计量契约

状态：部分调用映射已核对，快照恢复原因正在整改；完整真实链路未验收。
对应 MOD-08/09/15、P02/P04/P05/P08、E2E-03/04/10。

## 当前所有权与预算边界（2026-11-02）

参见[实体与所有权](../10-Goal任务与子代理/AGENT_THREAD_ARCHITECTURE.md)。canonical transcript、模型工作集、压缩摘要、物理请求快照和 usage ledger 分离。冷恢复的 32 条/32KB 是初始选择窗口，不是原文可访问上限；父子只传 brief 与受控引用，memory/CodeMemory 访问仍由各自 policy、工具和资源授权决定。

当前 request_model 在完整上下文与工具 schema 编译后估算，压缩返回后仍走同一硬预算检查；不能把下面保留的历史超限原因理解为当前缺少压缩后检查。手动维护在 idle 准入，provider context-limit recovery 从内部模型边界进入；二者计划预算与阈值压缩仍有差异，但不绕过后续请求的硬检查。估算加 provider 校准不等于精确 tokenizer 上界。

## 四种数据不是同一份历史

| 数据 | 权威与寿命 | 输入 → 输出 | 失败语义 |
|---|---|---|---|
| 原始 transcript、Goal/Task/权限事实 | canonical Store；按产品保留策略 | 已提交事件 → 原文/状态引用 | 不可用时不能让摘要冒充原文 |
| 动态上下文工作集 | Core 计划，Application 提交 revision | 已压缩部分 + 未压缩 tail + 必需事实引用 → ContextPlan | 计划失败保留原工作集 |
| Provider 物理请求快照 | Host 派生、可丢弃、单个有界最近请求 | 已接受配置 + wire body + response witness → 可恢复热分叉候选 | 不可用走明确原因的冷恢复，不影响原文完整性 |
| usage ledger | 已提交 physical attempt 事实 | provider usage/缺测事实 → 会话累计统计 | 缺测是 unknown，不补 0、不估造 cache read |

实际入口：`cli/src/llm_parts/request_snapshot.rs` 的 PreparedModelRequest 保存实际 body、
完成响应、配置及开始时间；`cli/src/agent_host_driver_parts/compaction_summary.rs` 的
CompactionRequestWitness 保存会话/配置身份、窗口及 usage，文件为 `compaction-request.json`。
它不是 checkpoint 或原文备份，删除它最多损失热路径资格。

## CTX-01：请求与配置冻结

### 会话累计计量的模块边界（2026-10-15 实施）

- Domain `ConversationCacheUsage` 封装加权累计和缺测覆盖规则；输入是当前协议的物理 Attempt usage，不接受 UI 估计值。
- Store `conversation_cache_usage` 在同一读快照合并该会话的 Run、Conversation owner 辅助 operation 和已验证远端观察。
  辅助 operation 按 `operation_id + attempt_id` 的唯一账本行读取，late usage 覆盖原行，不能把通知或重放当新请求。
- 远端 physical Attempt 仅在执行 authority 记账，本地保存有明确本地 owner 的观察引用；同一来源同时有本地账本时不能重复加总。
  Account/Task operation 不凭全局模型或当前 UI 会话猜测归属，不混入其他会话累计。
- 正文上下文占用仍取最新已提交 Run；缓存命中率取会话全部历史的 `sum(read)/sum(input)`，二者寿命不同。
  缺测请求只增加 `unreported_requests`，不补零；没有已报告请求时命中率未知。运行中/中断的不确定请求也保留覆盖缺口。
- UI 只展示 Application 的累计投影，切换/压缩/重启不能重置或自行累加。Provider 未提供货币计量时费用保持未知，不按 token 数猜造账单。
- iOS 上下文占用与累计缓存独立展示：没有已提交占用时显示“暂无已提交的上下文用量”，不画伪造的 0% 进度条；
  明细估计失败也不能隐藏已持久化的缓存率与计量覆盖数。遵循 Apple HIG 的反馈、进度指示与辅助功能原则，使用语义色和自适应文字。
  普通读取不触发网络；仅显式刷新按原 Host/operation 查询迟到回执，取消编辑不触发远端刷新。

输入输出：物理账本/远端观察 → Store 快照聚合 → `ContextUsageView.conversation_cache_usage` → 所属会话的客户端展示。
错误：无 owner 不生成统计；错误来源绑定拒绝；数值溢出不生成伪百分比；缺失 provider 计量不是请求失败。
本节是当前整改契约，不代表已经通过统一编译或真实模型验收。

每次模型 Attempt 必须有 run/request/attempt identity、目的、协议、有效模型配置 revision、
context revision、tool catalog revision、开始/结束状态与 usage coverage。
会话显式配置覆盖全局；未设置、明确禁用、协议不支持分别表达。物理 fallback route 也是独立尝试，
不能把它的 usage 归到未实际调用的 primary，也不能用冷摘要偷偷换小模型。
凭据只通过安全引用解析；快照不存明文密钥，诊断不打印 body 或凭据 digest。

热分叉使用已捕获的物理请求，不能从逻辑 transcript 重新序列化来宣称前缀相同。
tools/system/model/thinking 与已发送历史保持不变，末尾追加维护指令。
是否在缓存有效期内只能作为客户端资格估计，最终是否命中由 provider usage 证明。

工具 schemas 由 Core 单一编译入口同时生成实际有序数组和 Step witness；Host 不再接受
仅集合相等但顺序不同的数组。实时 activated catalog 不能回填历史 ToolResult。
具体职责与待验收项见 [Harness 上下文投影](HARNESS_CONTEXT_PROJECTION.md)。

## CTX-02：压缩事务

### 窗口、输出额度与工作集目标的准入约束

物理上下文窗口 W、已准入输出额度 O、安全余量 S、输入硬预算 P、压缩触发
阈值 T 是不同量。P = W - O - S；必须先验证非负及当前必需输入可容纳，再令
T ≤ P。不得通过 saturating_sub 后 max(512) 把不可行配置伪装成可用预算，
也不得把 O 悄悄改成 W-512，却继续声称它等于物理请求的输出额度。

Application 冻结配置时区分显式输出额度与继承默认：显式值与窗口不相容时，
返回结构化配置冲突，附 W/O/S/P，提示调整相冲突配置，不发摘要请求；默认值
如需按模型窗口适配，必须在配置准入阶段确定，并同步到普通请求及摘要请求。
不能由 Core 压缩器单独降低输出上限，也不能在热分叉时更改已捕获请求的额度。
工作集目标只是提前压缩策略，不得当成模型物理窗口去截断输出或静态前缀。

必需输入已超过合法 P 时，区分不可拆的当前输入与可压缩旧历史：前者明确返回
输入不可容纳，后者重选压缩范围并预留摘要。该错误不是模型摘要失败，也不是
验证证据不足，禁止进入无限摘要重试。原文、用户提交和已有工作集保持不变。

已修复的配置缺口（2026-10-12 Mac 实测）：W=16000，O=15488，P 被夹为 512，T=512，
摘要目标 307；配置在准入阶段未被纠正/拒绝，最终由 summary_token_budget 报错。
因此不能将本样本归因于“尾部过长”。后续已接入配置准入和实际 wire 输出额度，
同一 16000 窗口真实 Mac 用例通过，但尚不证明完整请求预算成立（见下）。

整改接线（已完成 Mac 流程复验）：Domain `resolve_output_limit_with_window` 统一计算；
未设置输出额度时，默认额度取模型输出能力、产品默认与窗口可分配空间的较小值，
窗口分配沿用 60% 输入工作集目标并扣除安全余量，不把该比例称为性能达标预算。
16k 样本得到 O=5600、S=800、P=9600。显式额度只校验、不重写。
HostOutputAdmission 当前版本为 6，保存 configured_context_window 与解析来源
ContextWindowDefault，恢复重新核验冻结事实；不兼容或迁移旧版本快照。
普通生成和摘要沿用同一已准入输出值，热分叉不临时调整额度。CLI 预算入口保留
W-O-S≥512 的准入保护；实际必需输入能否放下仍由请求规划继续核验。

### 历史超限问题与当前完整请求准入

2026-10-12 `/tmp/nola-mac-real-memory.xx14Ue/runtime/provider.ndjson` 的真实用量
显示：带 18 个工具的正常请求输入为 16672–21605 token，已超过配置 W=16000，
更超过 P=9600；O=5600 已正确上线路。这次服务端接受请求只能证明任务流程可跑通，
不能证明客户端窗口预算正确，也不能称为严格 16k 模型验收。

历史版本定位：`runtime/model_planner.rs` 编译完整工具投影后调用
`refresh_model_estimate`，但仅在 `automatic_compaction_count == 0` 时触发压缩，
之后即发 InvokeModel，缺少压缩后的硬预算准入。`compaction_driver.rs` 又按
model_context 单独 rebase，摘要预算没有包含这次完整模型请求的静态与工具成本。
`item_token_estimate.rs` 使用字符数/4，不能当作真实 tokenizer 的准确上界；
usage 校准存在，也不代替发出请求前的硬预算判定。

整改必须整体遵守以下契约，而不是再提高测试窗口或无限压缩：

- Core 在完整请求编译后保留静态前缀、工具、保留历史、待压缩历史的预算拆分；
  压缩规划消费同一份事实，不能用少算工具的预算证明候选可放下。
- 自动压缩次数只限制维护重试，不解除请求预算约束。压缩后重新编译、估计并准入；
  若必需输入仍不可容纳，产生明确上下文受阻结果，不发模型请求、不回滚已保存原文。
- 压缩触发阈值与硬窗口分别测试。真实 provider 用量若超过窗口，验收必须单列失败，
  即使任务返回成功；未获得实际 tokenizer 时，应披露估计误差及校准覆盖范围。

当前实现进度：自动阈值路径在完整请求编译后，将 P 和目标额度按已有 provider
校准比例反算回历史估计单位（向下取整），扣除完整请求估计与历史估计之差，
再传给压缩规划。该差额预留工具 schemas 与额外指令；不再在历史 rebase 时丢失。
若无剩余额度或保留历史使摘要无处容纳，记录 context.input_budget_exceeded
受阻事实，不追加摘要请求；摘要返回后仍重新编译并检查完整请求预算。
这不是精确 tokenizer：首次请求无校准时仍有估计误差，且手动压缩与服务端
context-limit 恢复路径尚未接入同一完整请求预算，不能宣称所有入口已经统一。
若扣除非历史成本后软目标为零，摘要上限回到硬剩余空间，并继续受原文减量约束；
不得把不可达软目标变成 1-token 摘要合同。真实 64k/30k Mac 压缩复验已覆盖此路径，
实际输入均小于硬输入预算，压缩后原文召回成功，见 VERIFICATION 的 nNR5lx 记录。

Core 确定待压缩范围、来源版本及窗口预算；Host 不能自行截掉源历史以适配请求。
热请求保留原前缀，同时明确总结范围；冷路径可按来源顺序分段滚动汇总。
摘要输出不另设任意 4096 限制，遵守模型有效输出配置与实际窗口余量。

```text
计划提交 → 快照/窗口资格检查 → 热分叉或冷计划
    → 完整摘要候选 → 来源 revision 核对 → 原子提交新工作集
    → 有正文但截断：保留事务内片段，原请求末尾续写 → 完整候选
    → 空/非法工具调用/不可恢复错误/取消：保留旧工作集及原文
```

维护响应中的工具调用绝不执行；续写片段不能提前覆盖 canonical summary。
成功提交必须包含摘要、source fingerprint、原文范围引用、lineage、context revision、全部请求计量。
并发新消息或 Goal 更新不得被旧压缩覆盖：核对来源 revision，不匹配则拒绝旧候选或显式重计划。
当前 Core 的具体提交见证是 `PendingCompactionPlan.working_set_item_count` 与
`working_set_fingerprint`：对计划时完整模型工作集做有序内容摘要，另存选定 archive 的
ID 顺序与内容摘要。计划不再保存第二份 `kept_items`。完成时先核验当前工作集的原始前缀，
只移除选定 archive，从当前工作集保留其余项及计划之后新追加的输入；新尾部同样进入
prompt 窗口预算。原始前缀被改写、缺失、重复 ID、archive 来源不匹配都拒绝候选，
不提前消费 pending effect、响应 cursor 或原文。该契约随当前 checkpoint schema 16
持久化，没有旧 plan 默认值回退。Goal/Task/权限事实仍由各自规范记录管理，不能由摘要覆盖。
本项已修改源码，尚未运行最终统一验收。上述 schema 16 是该压缩计划变更时的
版本；当前 checkpoint 为 19（含独立控制拒绝、许可复用来源；恢复状态为必填 tagged enum），仅接受
当前版本，不存在 16 的兼容解码或升级路径。
压缩不拥有 Goal/Run 终态；窗口仍不足时返回结构化 blocked/incomplete，由统一收尾处理。
原文召回按 canonical 会话授权读取，BM25 是可重建索引，不能因为索引缺失删除原文或授予权限。
摘要中的外部文本仍是数据，不提升为用户/系统指令。

## CTX-03：快照落盘、恢复和失效

当前落盘：同目录唯一 staging，Unix 0600，文件 fsync → rename → 目录 fsync。
单个快照读取上限 32 MiB 是派生缓存资源预算，不是摘要输出上限。
读取必须自身有界，不能仅 stat 后无界读取；校验 session/config/request、窗口及时间资格。
失败不修改原文件，不把损坏快照当可执行请求。

恢复结果必须区分：

| 分类 | 当前稳定指标 code | 行为 |
|---|---|---|
| 不存在 | no_request_snapshot | 冷恢复 |
| I/O 读取失败 | request_snapshot_unreadable | 冷恢复；诊断不泄露内容 |
| 超过缓存读取预算 | request_snapshot_too_large | 拒绝读取超限内容，冷恢复 |
| JSON/外层字段无效 | request_snapshot_malformed | 冷恢复 |
| 会话身份不符 | session_changed | 拒绝跨会话快照，冷恢复 |
| 配置身份不符 | configuration_changed | 使用当前准入配置冷恢复 |
| 内层物理请求恢复无效 | request_snapshot_invalid_request | 冷恢复，不猜测旧协议 |

读取成功后还需 fork 检查年龄、前缀覆盖与窗口，不能把“文件可解析”当作热缓存命中。
内层 invalid_request 当前仍合并协议/route/时间/字段等原因，后续需细分；不宣称观测完整。
快照写入失败也要有独立指标及生命周期证据，目前尚未完成该路径审计。

## CTX-04：Responses 历史缓存边界与实时事实

普通续轮保留原始会话/工具历史，在尾部重新投影当前权限、工程状态和执行预算；旧尾部是上轮请求的临时投影，不为命中缓存持续累积旧授权快照。Core 在第一条当前权限投影标明 `_nola_cache_tail`，共享 Rust Responses adapter 可在此前的持久历史文本块加显式缓存边界，实际 wire 移除内部标记。角色、文本、顺序、Core 准入与冻结权限均不改变。

该端点能力使用现有显式配置 `llm.openai_cache: {"enabled": true, "history_breakpoint": true}`；只在实际验证支持 `prompt_cache_options` / `prompt_cache_breakpoint` 的 Responses 端点启用。不默认对所有兼容端点强开，不改 SIWC，不与显式 retention 混用，也不强行固定 key。`false` 或未启用时保持普通协议请求。父子冻结配置携带相同开关，恢复不从当前全局选模重建旧策略。 多连接的日常配置使用 `llm.openai_cache: {"enabled":true,"history_breakpoint_endpoints":["https://api.openai.com/v1"]}`，不同时设置全局 `history_breakpoint`。列表按实际请求端点（含 fallback）匹配，未列出的端点不发送扩展；列表随父子和会话执行策略冻结。空列表关闭全部端点的历史边界，不靠模型名称或供应商猜测能力。

使用新造的工具历史实测：无边界时替换尾部事实可使整段历史冷启动；同一 key/schema 配置边界后，可复用边界之前的历史，仍将最新事实原样置于其后。是否命中以每次 provider usage 为准，不能由公共 JSON 字节数推算缓存 tokens。工具集合、真实前缀、压缩/配置变化仍可导致冷启动；这不是原始 transcript 或权限状态的另一份权威。

## ACC-01：会话累计缓存率

实际类型：`core/nola-domain/src/context_accounting.rs` 的 PromptCacheUsage、ConversationCacheUsage。
实际聚合：`core/nola-store/src/committed_run_context_usage.rs` 的 conversation_cache_usage，
按 run_events 中 `(run_id, attempt id)` 取最后一版，统计确实开始/有 usage 的请求；在途或中断而缺测的请求保留 unknown coverage。
这不是 Trace 推断，但需继续证明各协议的所有物理尝试（包括失败、fallback、压缩续写）都进入此 ledger。

公式：可比较且有效的请求 `sum(cache_read_tokens) / sum(total_prompt_input_tokens)`。
不平均每条百分比；unknown 请求单列 reported/unreported coverage，不混作 miss。
不同 provider 的 input 定义先归一化；如原字段不含 cache read/write，不能直接用它当分母。
read > input、计数溢出、缺测分别导致率未知，不能变成看似合理的百分比。
估算上下文占用不是已计费输入量，不能用来计算实际 cache 命中或费用。

CommittedRunContextUsage 当前只接受协议 2（存储版本以 `STORE_SCHEMA_VERSION` 为准，无旧协议回退）：携带单 Run 累计
`run_cache_usage` 和独立的 `attempt_sequence`，以及原有上下文 `event_sequence`。
两条前沿来自同一 SQLite 读快照；后者不增长时，迟到的请求 usage 仍可推进累计统计。
摘要只包含最后一请求的 cache_usage 不足以恢复累计，禁止拿它冒充累计。
接收方先验证 source epoch/host/Run/根消息 admission，再按 Run 替换持久快照而非累加。
两前沿相同而内容变化、交叉前进/倒退、某一前沿不变而对应事实变化均拒绝；旧快照不覆盖新值。
本地 Run 从物理 attempt ledger 聚合；远端 Run 只从已接收累计快照聚合，不重复计入本地投影事件。
跨 Run 汇总加 token 分子分母和 coverage，不能相加或平均百分比。
它仍是单 Run 的使用量投影，不自动等于跨 Run 的会话累计。
本地重启、远端同步和 UI 切换都须消费正确 scope 的统计；不能把“当前一轮 0%”标成“会话 0%”。
Canonical ledger 与显示值需断言同一 revision，迟到 usage 更新应幂等增量生效。

## 验收与测量

### 手动维护与活动执行的准入边界

公开 `AgentSubmission::Compact` 是空闲维护请求，不是中断或排队命令。
Core 在任何非终态 Turn 或尚未静止的 Host effect journal（包含缓冲终帧）存在时
返回明确的 `manual compaction busy` 拒绝：不写接受回执、不创建维护 Turn、不发
`CompactionFailed`、不修改原权限／输入等待或取消收尾。调用方保留原执行状态，
在其真正收尾后才可显式重试；不能以拒绝为由重新发起原用户任务。
Application 的 active Run 检查是外层保护，不能替代 Core 对直接 submission 的保护。
自动阈值／context-limit 压缩仍从内部模型边界进入，不经此手动维护准入；取消摘要后的
晚帧沿原 effect 的取消协议处理，不能恢复模型调用。
回归源码覆盖模型在途、权限等待、取消收尾，以及终态标签但 effect 尚未收尾；
拒绝前后整个 checkpoint state 必须相等，原模型／权限回执仍应可提交。

快照测试必须覆盖读失败分类、身份拒绝、有效恢复、超限、冷路径完成，以及 fallback 指标的实际传递。
mock provider 只能验证协议与状态，不证明缓存真的命中；真实模型验收核对 wire prefix 与 cache read。
压缩耗时从计划到 context commit，包含拒绝、分段、续写，不能只报告最后一次请求。
首 token 分为 reasoning/text，与配置、准备、provider 等待、投影首帧分开；spinner 不算 token。
在 VERIFICATION 中登记环境和多样本基线后才定延迟预算，不以一次短会话替代生产长历史。

## 模型上下文的快照与来源契约（2026-10-28源码核对）

Application ContextCheckpoint不是聊天列表缓存：它标明conversation、可空Run、model_profile、policy_version、覆盖到的message/sequence、summary part、tail起点、token_usage及payload_digest。一次实际模型请求必须能解释它采用哪个工作集，而不能只保存“当前token数”。

每个ContextComponent同时携带kind、source id/URI、authorization、observed_at/max_age、freshness、content reference/digest、estimated tokens及provenance。provenance包含provider、采集方法、source revision和是否disclosed_to_model。授权状态与新鲜度是独立维度：数据未过期不等于已授权，授权有效也不代表内容仍新鲜。

流式模型输出、消息原文、上下文摘要和长期记忆各有归属。压缩不能覆盖原文；summary/tail覆盖范围应能从checkpoint重建，跨会话不得按“最近摘要”猜测。模型请求实际usage与Core估算分开保存，measured缺失不得变为精确0，UI只能显示当前证据支持的度量。

恢复必须沿原执行配置与context来源核对；用户切换模型或成员不会给旧Run移交其他Context。生成摘要是物理模型操作，Host返回候选与usage；采纳由共享Core/Application判断，不允许Web/Apple/Android各自生成并写另一套业务压缩结果。

### 原生维护调用的模型归属

一次 summary/compaction 开始时冻结完整执行配置，所有分段 continuation 复用同一模型、连接、能力和 generation snapshot；切换全局模型只影响后续操作。Core AuxiliaryInvocation 保存 profile revision、configuration digest 和逐次回执，不保存凭据。Android 复制远程 provider 配置；iOS 捕获不可变 provider；共享本地引擎变更时明确中止该维护而不改投新模型。远端 Host 在第一次 prepare 后冻结源 profile/revision，源 Host ledger 才是物理请求权威，本地不得重复登记虚假的物理 attempt。


### 长历史的软目标分配（2026-11-03）

完整请求的软目标不等于摘要硬上限。Core 先反算校准并扣除必需前缀与工具成本，
得到历史软目标 Hs 与历史硬容量 Hp；历史当前估计为 H。共享 Kernel 采用
`min(Hp, max(Hs, ceil(H/4)))` 作为替换工作集目标。近期完整事务与摘要共享此目标，
保留最新用户锚点和工具配对；摘要另扣保留项与 framing，并仍受归档净缩上限限制。
这避免必需前缀接近软目标时，把数万 token 历史强压到偶然剩下的几百 token。
它允许超出软目标，不允许超出硬容量；预算不足仍明确失败，摘要绝不截断或跳过校验。

比例针对当前 `model_context`（已有摘要与尚未压缩内容），不是 lifetime transcript，
重复压缩不会因原文累计而无限增加预算。旧摘要层合并、来源引用、原文保留及
压缩后完整模型请求重新编译/准入保持不变。手动维护、阈值维护及溢出恢复均由
同一 Core 分配；真实 provider 校准仍只是估计，不是 tokenizer 上界保证。


### 物理响应与投影失败的用量（2026-11-03）

Host 取得一份完成的物理响应后，先通过现有 `ModelUsageReported` 提交精确 Attempt
用量，再发布 provenance、正文与工具调用。后续投影被拒不撤销这份已发生用量。
完成事件可带逻辑轮次的累计用量；Attempt 账本已有物理快照时不得被该累计覆盖，
避免恢复/纠正请求被重复计费。ContextWindow 的工作集计量不等同物理费用账本；
缺测仍为 unknown，不用成功消息或当前 checkpoint 次数推算真实请求数。
