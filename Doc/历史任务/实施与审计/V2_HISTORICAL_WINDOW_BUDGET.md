> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 历史语义窗口预算与明确完整性

2026-09-08，本轮实际接线记录。

窗口与context仍共同调用 `Application historical_part_projection::read`。Store提供typed `HistoricalPartRead::{Absent,Available,BudgetDeferred{canonical_kind}}`，预算耗尽与原文/owner/revision错误分离，不通过匹配错误字符串降级。Absent只表示规范JSON确实不存在。

多结构化Part耗尽selection的32768-byte语义预算后，窗口保留所有canonical Part节点，以 `HistoricalProjectionStatus::NotLoaded{reason:SelectionBudget}` 明确整份语义未加载：owner/canonical revision/kind来自已验证metadata，source/schema/binding真实None，fields为空。它不是Deferred字段、不是Absent、不是Unsupported。仍按原Message ordinal分页，不用has_more伪装Part内分页。每个window完整编码（包括所有占位、IDs、text、live节点和分页字段）由streaming计数限制为2MiB，与native projection FFI上限一致；metadata自身超过该上限仍明确报错，尚未实现Message内部metadata分页。

新增公开fresh query `read_historical_part`：

- 参数 `owner:{conversation_id,message_id,part_id}`、`expected_conversation_revision`、`maximum_output_bytes`（1024..32768）。
- 返回 `Option<HistoricalPartProjection>`；预算不足返回NotLoaded，连最小规范metadata也装不下则明确query错误，不能超出请求预算。
- 真实owner、可见性、revision与原source核对仍由Store执行；查询不持久化receipt。源错误不会转成NotLoaded。

五端decoder已识别not_loaded。reader新增 `canonical_text:{text,total_utf8_bytes}` 后，五端在历史说明中展示原规范文本预览，包括Unsupported的opaque描述；不会变成Tool/permission执行输入。共享fixture `historical_unloaded_conversation_projection_v1.json` 覆盖NotLoaded真None/空fields与Unsupported canonical_text，Swift/Kotlin/Node/CLI测试消费相同fixture。

**最新交互进度**：macOS/iOS/Android/Web 的实际历史详情入口及 CLI `session part` 已接通，采用独立详情状态，不替换规范stream节点。身份/预算/取消/查询无receipt已有专项证据，详见 [历史详情交互](V2_HISTORICAL_DETAIL_INTERACTION.md)。完整页面点击、截图和超大原字段range下载交互仍未完成。下文较早的待验证记录由该文档最终结果覆盖。

测试覆盖：多Message24Part累计超过32KiB仍全部保留，原消息分页边界稳定，逐Part独立查询恢复；escaped owner/kind使最小metadata超过1024时明确拒绝；真实Service64Part预算→单Part查询、错误owner/revision、冷重启、无receipt由root复验。各端原生编译/测试结果由root统一记录。

## macOS 按需详情接线（原生验收待本轮终态）

macOS 实际 `projectMessages(window)` 给历史 Part 生成只含 canonical owner、canonical revision、window event sequence 的选择描述符。消息行的“查看历史详情”（`chat.historicalDetails.open`）打开独立详情 sheet；多 Part 可切换选择。详情模型请求正式 `read_historical_part`，固定32768-byte预算，Port在解码前拒绝更大的response，并核对返回owner/revision。已有window与live projection不被合成replace，详情响应不写canonical消息或修改事件序列。

窗口选择描述符改变或消息行离开时关闭详情；独立模型以generation丢弃取消、换会话、窗口失效后的迟到响应。详情用纯文本展示完整字段，Deferred字段明确未加载；NotLoaded/Unsupported保留Core状态。不能把历史JSON构造成工具调用、权限批准或附件打开动作。读取失败提示关闭详情并刷新会话，不静默使用新revision重试旧owner。

新增 `MacHistoricalPartDetailTests` 四项：raw响应上限与owner/revision验证，异步迟到结果及窗口失效，Deferred/NotLoaded明确保留，以及真实Service64Part初窗预算耗尽后按需加载末Part且window/projection/sequence保持不变。Swift parser已通过，原生编译/测试与截图由root记录。五端交互尚不能由macOS单端实现推断完成。

### Mobile on-demand interaction

The iOS `ChatBubble` and Android `MessageBubble` now expose a historical-details action for each canonical historical Part. A separate sheet/dialog reads `read_historical_part` with the original canonical owner, conversation revision, and 32768-byte cap. The response never enters the live reducer, tool permissions, or sequence stream. Replacement of the window's historical Parts closes the selection; cancellation and response owner/revision/kind validation prevent a late read from replacing another detail selection.

The detail view displays loading, unavailable/error with retry, complete descriptive fields, and explicit Deferred/NotLoaded states. It does not claim that an oversized original field has been loaded and does not execute historical JSON. Android retains the action metadata only in a serialization-transient UI field. The existing versioned mobile submission adapter classifies this command as a fresh query before durable command admission; no receipt is stored for it. Native validation for this addition is coordinated by the root agent; Swift syntax parsing succeeded locally.

## Application 历史读取 / Context / Revert 的 Port 切换

2026-09-08 增量：`conversation_projection.rs` 的 full/page/window/usage/fresh-Part 入口已使用 StorePort 泛型；`canonical_branch.rs` 的首次上下文恢复与 revert-prefix 恢复使用 `CanonicalHistoryRepository`；`historical_part_projection.rs` 与 `durable_conversation_items` 只依赖 `HistoricalPartProjectionRepository`。这些生产实现不再接受具体 `NolaStore`。完整投影入口复用 local presentation subset / remote projection subset，不为查询恢复无关 full Session checkpoint、权限或其他业务集合。

StorePort `conversation_history` 承载原 StoredPart/Message/Conversation/Window DTO；`CanonicalHistoryRepository` 只增加有界窗口选择，`ConversationProjectionRepository` 组合历史语义、现有 Run/Journal 端口和presentation元数据读取。Store旧DTO名称原样reexport。Sqlite adapter直接转发现有同连接实现，不打开新库或引入新transaction；aggregate read仍受现Application一致读边界保护，单字段owner/revision/source检查不变。

`revert_conversation` 改为 CanonicalHistory + RunEffect + ConversationRevert Port 泛型。原 checkpoint/projection提交DTO迁入StorePort，`ConversationRevertRepository::commit_checkpoint_projection` 映射原atomic实现，保持outer receipt事务内的savepoint与revision CAS。没有clear→create或整JSON回退。

验证：无SQLite测试2/2使用实际App窗口/full projection生产入口，64Part保留NotLoaded并验证stale source错误，fake中的checkpoint加载/任何写入均panic；branch3/3；bounded revert receipt1/1（终态receipt注入失败后checkpoint、visibility、journal、sync、revision全回滚，重启/replay成功、10MB原文不变）。App lib85/85通过。日志`/tmp/nola-history-port-tests.log`与`/tmp/nola-history-port-app-lib.log`，所有句柄终态。这是完整历史读/回退切面，不代表Application其他命令已全部移除Store依赖。
