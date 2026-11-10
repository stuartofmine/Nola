# 当前 Core 与 Host 接口

基础字段记录：2026-10-28。2026-11-08 重新核对总体接口索引、制品及模块所有权；下方专项字段仍保留其原核对范围，未重新执行每个 handler 的动态验收。本页描述已有接口，不引入新服务或新协议；不构成所有平台全部验收通过的声明。

协议或存储格式不匹配必须明确拒绝；不提供旧字段别名、双读双写、迁移或自动重建数据。A2UI 固定 v0.9.1；HTTP 路径中的 v2、SubmissionFrontier 的 protocol_version=1 与 A2UI 版本属于不同协议域，不得混用。

## 当前实现导航（2026-11-08）

- [接口源码索引](INTERFACE_CATALOG.md)已按当前字面量路由及 command metadata 重新提取；不能把先前 235 个 tag 的记录当成当前全部命令。
- Service 的 `ProtocolFailure` 当前是 code/message/recoverable/details?；receipt outcome 只有 accepted/committed/rejected/conflict。传输不可用/提交未确认不是新的持久 receipt tag。
- 最终 Application C ABI 在 nola_mobile_bindings；nola_core 是 rlib facade 与纯辅助 ABI 实现之一。具体 schema/ABI/Service 版本取各自源码，不跨域比较数字。
- 已发现的记忆 owner、bootstrap 历史配置恢复与声明耦合差距见[源码核对](../../总体设计/ARCHITECTURE_SOURCE_AUDIT_20261108.md)，不能因 DTO 已存在就标记业务闭环完成。

## 权威与调用边界

| 层 | 当前职责 | 输入与结果 |
| --- | --- | --- |
| Core/Application | 准入、恢复、权限、Goal/Task 状态与动作裁决 | 类型化命令，原身份及版本；返回已提交回执或明确错误 |
| Store | 原子保存 checkpoint、witness、事件、outbox、投影及复制记录 | CAS 与不可变摘要校验；不把镜像变为源执行权 |
| Host/driver | 执行已 claim 的模型/工具，续租、采集平台确认、报告执行事实 | worker/generation、effect/attempt/token 与原 Run 绑定 |
| Renderer | 消费 Core/A2UI/journal 投影并提交用户操作 | 本地保留草稿、IME、焦点、滚动和动画；不另建权限或任务状态机 |

Apple 复用 Shared 协议与 SwiftUI 渲染；Android 使用现有 JNI，Web 使用 Rust Host HTTP/journal。A2UI 接入不新增 UniFFI 或浏览器 WASM Runtime。

## 源提交前置条件

`POST /api/mobile/chat/agent/v2/submission-frontier`：Bearer 配对鉴权，使用现有移动加密 JSON 信封。请求为 `SubmissionFrontierRequest`：

| 字段 | 约束 |
| --- | --- |
| expected_host_id、client_id、conversation_id | 非空、无首尾空白/控制字符，至多512字节；Host必须匹配 |
| expected_run_id、assistant_message_id | 可空，但必须同时提供或同时缺省；已有历史时提供原准入 owner |

成功HTTP 200的解密载荷为 `{source_host_id, result}`；result为 `SubmissionFrontier`，包含 `protocol_version=1`、源Host/client/conversation、可空 `configuration_revision` 与 `parent_message_id`。未认证401、请求格式错误422、无法确认源前置条件409（`admission_frontier_unavailable`）。

不存在会话可返回空配置/父消息；已显式选模但从未有消息或Run的本地User会话允许读取初始配置版本，即使已有ConfigurationChanged检查点。是否有执行历史由包含隐藏消息及已封存Run的存在性查询决定，不能按checkpoint是否存在推断。已有历史不允许无owner读取执行前置条件。

客户端核对源身份与当前显示父消息，把返回值冻结为后续提交的 `expected_configuration_revision`、`expected_parent_message_id`，纳入原提交摘要。绑定后的重试沿用原值，不重新读取后伪装成同一提交。父消息不一致时保留草稿并重新加载历史。显示用的后继消息ID不能替代原Run的assistant根身份。

## 会话执行模型选择

`POST /api/mobile/chat/agent/v2/execution-selection`：同上鉴权与加密信封；请求严格拒绝未知字段。

```json
{
  "expected_host_id": "host-id",
  "conversation_id": "conversation-id",
  "action": {
    "type": "select",
    "expected_revision": 2,
    "selection": {"kind": "model", "profile_id": "profile-id", "profile_revision": 3}
  }
}
```

- action为 `read`、`select` 或 `create`。select携带配置CAS版本；create携带title与selection，复用Core原子创建/初始配置提交。
- selection为 `{kind:inherit}` 或 `{kind:model,profile_id,profile_revision}`。源Host检查profile当前版本、enabled、未删除、remote运行类型及chat能力。
- 修改的是该源会话配置，不是Host全局默认值；远程镜像不能在本地冒充源配置提交。
- 成功解密载荷为 `{source_host_id,conversation_id,result}`。read可返回read/not_found；select返回Core配置提交结果；create返回created/already_exists。客户端必须核对result状态，不能仅凭HTTP 200判断已修改。
- 未认证401；格式错误422；源/配置无法确认409，code为 `conversation_selection_unconfirmed`。

## 移动本地子任务 Application 命令

以下为Application命令tag，不是独立HTTP路径；所有字段沿用当前Rust类型。

| tag | 输入 | 输出与约束 |
| --- | --- | --- |
| load_mobile_task_wake_plan | now_ms | observed_at_ms、next_wake_at_ms、candidates；候选含task、父/子conversation、parent_invocation/config。远程复制投影不能授予本地执行权 |
| admit_mobile_task_child | input: task_id、parent_conversation_id、worker、now_ms、lease_until_ms、candidate_config、current_tools | task/parent、attempt、cancellation_only、invocation、current。Core核对原冻结绑定；不是客户端自行创建child Run |
| inspect_mobile_permission_request | 原HostRequestEnvelope、now_ms | 原已启动审批的展示事实；只接受同attempt/token/worker的已提交续租，不产生许可 |
| recover_expired_run_deliveries | conversation_id、expected_run_id、host_invocation、interrupted_at、interrupted_at_ms | 核验原Run与Host调用后恢复到期delivery；不能抢有效lease |
| prepare_run_effect_recovery | input: conversation_id、run_id、invocation_config_digest、now_ms | rebound_count、Core traces；重复准备不重复授权/执行 |

current_tools是当前Host工具观测，Core按冻结child集合收窄，只允许合法receipt刷新；schema、权限和执行绑定不能扩大。原生foreground与embedded执行器应复用同一工具plan重验证。

child pump启动及后继claim前复用恢复命令。Task lease与effect lease到期不同时不能漏恢复。已bound且持久cancel_requested的child可获cancellation_only准入：沿用原绑定推进取消及精确终态回执，不普通claim、不新增模型尝试，也不由Host直接伪造Task完成。

退出页面、取消观察协程、Activity销毁及流关闭只detach本地观察/driver；用户显式停止才提交业务取消。取消已请求不等于外部效果已停止，必须等canonical终态及精确证明。

## 审批恢复与版本

`PendingPermissionRedisplay`是Core恢复证据，不是用户Allow，也不是通用风险重试授权。只有恢复事务可在核对原不可变outbox摘要、未封存Run、当前checkpoint witness、原pending审批后使用；原请求可重示，新投递仍须新的合法owner，旧owner回答拒绝。

Core revision记录流式reduce推进，Store checkpoint revision记录持久提交CAS，两者不相等。permission settlement的source revision须落在前后Core witness区间 `(previous_core_revision, next_core_revision]`，并满足原turn、effect、owner与cursor校验；禁止以Store计数器代替Core版本或删除检查来绕过冲突。

## Goal 控制能力

`load_goal_control_observation {conversation_id}`返回可空 `GoalControlObservation`：goal_id、control_token、status、can_submit、can_pause、can_resume、can_cancel。客户端按Core能力显示操作，不从自然语言、是否正在stream或枚举局部猜测能力。控制token绑定Goal/control_generation，不等于普通projection revision；操作使用既有持久控制意图与回执，丢响应后先核对原操作，不能换ID重发。

Task journal中的sealed是终态，只表示已封存，不自动表示成功。成功/失败/取消由canonical结果判断；TaskChild的只读边界依据Core execution_ownership，不能凭是否paired或是否配置模型推断。

## 远程内容同步与编码

观察原Run使用原源Host/client/conversation/Run/assistant身份；封存后待处理outbox可清理，冷读身份从canonical terminal等持久记录取得。先同步message admission，再同步context usage、journal及精确terminal frontier；传输结束或HTTP ACK不解锁业务完成。

`accept_remote_run_context_usage {source_host_id,record}`保留源完整记录及record_digest。外层snake_case不意味着嵌套payload也用snake_case：PromptCacheDiagnostic使用 `schemaVersion`、`identityStable`、`identitySha256` 等规范camelCase。Apple必须按DTO编码，不能让全局convertToSnakeCase改写嵌套协议；原键和摘要完整进入remote镜像。

内容准备/接收以原记录顺序和摘要为准。另一个receiver已提交同一记录时，迟到page只有在完整header、源授权、前缀/结果/目标manifest全部核对后才能幂等确认；不重建stage，不忽略未知冲突，不推进到另一个Run。记录尚未提交时仍必须是精确next record。

`GET/HEAD /api/chat/conversations/:id`复用既有详情查询，支持列表尚未缓存的child冷开；不存在会话404。Web本地会话接口不继承移动配对接口的鉴权说明，具体按当前routes及既有会话传输契约执行。

## 源码索引

- HTTP：`apps/cli/src/web_parts/{routes,submission_frontier,paired_execution_selection}.rs`。
- 类型/命令：`core/nola-domain/src/remote_submission.rs`、`core/nola-application/src/{application_command,mobile_delegation,effect_delivery}.rs`。
- 恢复/存储：`core/nola-mobile-bindings/src/child_pump.rs`、`core/nola-store/src/{effect_deliveries,permission_source}.rs`、`core/nola-store/src/assistant_journal/receive_content.rs`。
- 客户端：`shared/apple/Protocol/RemoteSubmissionIdentity.swift`、`apps/macos/Services/MacApplicationPort.swift`、Android `AndroidEmbeddedChildExecutor.kt`。

历史验证范围见阶段归档（本地工作记录，不随仓库分发）。本页核对不重新激活归档任务或清空的待办。


## 新原型共享查询与精确重发（当前实现，待统一验证）

| Application tag | 边界 |
| --- | --- |
| load_today_facts | 单一致读事务聚合本地普通会话、owned task及专家等待；独立全局recent、显式offset和queued分页；不授予恢复权 |
| admit_expert_conversation_message | 单写事务准入父消息、完整专家manifest及全部child；同intent幂等、不同载荷冲突 |
| load_expert_message_manifest | 读取精确父消息的完整集合与源发送见证；不以Room第一页或客户端当前选择替代 |
| prepare_edited_resend / prepare_edited_expert_resend | 核原消息与源配置见证，新建仅含编辑输入的目标；prepare结果不是已发送 |
| load_prepared_edited_resend / load_prepared_expert_resend | 查询准确operation/目标的真实准入；组状态必须同一读事务 |
| load_expert_child_instructions（仅可信 Host 私有操作） | 核当前 actor / Host、精确 child conversation / Run 和捕获来源后读取完整 typed 定义；不是旧资源授权的继承 |

`agent_room` 新精确子查询 `submission {room_id,submission_id}` 是只读；不能用分页第一页证明原child不存在。admit/prepare/claim依旧是写用例。精确字段以Domain类型和`ApplicationCommand`为准，当前数据不匹配明确拒绝，不补旧字段或自动改写记录。

可信Host私有 attestation 的唯一输入是 `{core_config,submission}`，证明后同一 submission 进入 Begin。C ABI为 `nola_core_application_attest_resend_host_policy`；普通 Renderer 命令JSON不能制造这个内部用例。Core共用typed组件校验，Store消费校验组件摘要和既有accepted-input digest；禁止证明后换正文、加入旧history或资源授权。私有模型重建材料不能通过通用product_settings读写/同步取得。

Web使用现有Host HTTP：`GET /api/workspace/today`，`POST /api/chat/expert-messages/admit`、`POST /api/chat/edited-resend/prepare`、`POST /api/chat/edited-expert-resend/prepare`；会话下提供对应prepared/source精确读取。请求核expected_host_id/client_id，实际执行复用现有source-submission、tool-plan、journal与终态协议。Renderer不得提交Core config、证明、凭据、模型替代或旧上下文。

`GET /api/chat/conversations/:id/workspace-review`只读当前可信本地Host工作区的有界Git状态及staged/unstaged diff；远程会话不获得本机同名路径读取。超时/超限是未完成读取，不是clean；没有新增Git提交/推送或虚构finding/gate接口。

可信 Host 的 `readable_templates` 私有操作读取现 `prompt.templates`，用现 resource access 策略逐定义核 `ReadContent`；不自动认领缺 owner / 缺资源 revision 定义，也不把 metadata-only grant 当内容可读。宿主构造真实安装 / 已认证 principal；Web 固定现 Host 本地 principal，不接受浏览器任意指定 actor。返回当前 customTemplates 的 `{revision,templates:[{id,name}]}`，不返回正文、资源授权、模型证明或凭据。当前时间由 Rust 提供。内部 `ListReadableExpertTemplates` typed variant 不接受普通 Application JSON 反序列化。缺失目录返回空集合；格式错误、重复 UUID、冒充内建定义或超预算正文明确拒绝，不重建目录，不新建平行目录。

`GET/HEAD /api/expert-prompt-templates`沿现Web localTrust/远程会话认证转发上述只读查询。专家管理仍通过`configure_agent`的directory/agent revision CAS保存`selectedPromptTemplateID`；实际发送时由Core在完整manifest准入事务内精确捕获定义正文。Apple内建模板不在此目录，不从缺失ID合成它们；Apple仍复用原共享模板服务提供内建定义。默认选项表示现继承语义，不能伪装显式None。未确认的专家编辑必须读到更高revision且完整目标定义一致才能确认成功，仅查到旧同ID不能丢弃编辑草稿。

### 专家定义捕获的私有 Host 入口（当前实现，移动端真实任务续验中）

上述专家 admit、prepare、manifest / prepared / child 正文读取均经 `HostExpertTemplateOperation`，不再暴露无身份的公共 Application JSON 入口。C ABI 为 `nola_core_application_expert_host_operation`，service 操作为 `host_expert_template_operation`，只有当前受信 native / server 工厂构造身份与内建定义。操作包括 `preview_capture`、`admit`、`prepare_edited_resend`、`load_manifest`、`load_prepared_resend`、`load_child_instructions`、`readable_templates`；没有旧 tag 别名。

`preview_capture` 在一致读快照内复用正式准入的模型解析，返回 participant、context configuration revision、defaults、原 profile declaration，以及精确所选模型的 `client_model_presentation`（无则 null）；不写 receipt、reservation 或 Run。presentation 仅来自该 revision 的 context 配置，不扩大 AgentDefaults 可写字段，也不返回其它模型的指导。正式准入重新核精确 model profile ID / revision 与 context revision，再同事务授权捕获完整专家集合。Host 用户指导以真实 producer 的完整 typed definitions 显式登记；实际 Begin 必须与 managed 定义及该登记集合完全一致，不能用 ID 前缀豁免。

私有捕获来源事实只证明当时合法取得的正文，不授予工具权限。保留定义的编辑重发要求当前可信 actor / Host 与精确原 witness；不得重采样当前模板替换旧定义，也不继承旧工具授权。Store 私有模型重建材料只返回 `expert_child` 事实，正文另走带身份与 exact Run 的私有查询。

传输 `request_id` 独立于业务 submission / operation ID。未知结果保留原私有操作和相同 request ID，经同入口对账；只有匹配该 request ID 且解码成功的确定终态才能清除 pending intent。需要重开 owner 的错误由 `nola_protocol::command_outcome_requires_owner_reopen` 统一分类。拒绝后新的显式尝试使用新传输 ID，可重新评估当前权限；已提交请求重试仍返回原提交事实。

service 与 embedded C ABI 复用 bootstrap `RuntimeOwnerGuard`：在原 writer mutex / 文件所有权仍持有时，先丢弃失败 composition，再打开同一个当前 Store。embedded 的多个 native handle / child pump 仍持同一 Arc，下一次获取 guard 可见新 owner；不能以单个 Swift / Kotlin handle free/open 冒充重开。重开失败保留 absent owner 并在下次获取时重试；不删除、转换或重建 Store。内存 Store 拒绝这种重开，避免换成空库。原 mutation 在 owner 不可用时维持未确认（service `Accepted`），不能生成业务拒绝后清掉原请求。真实 SQLite deferred-FK 提交失败、两个存活 native handle 的同请求恢复及一次提交、重开失败与内存拒绝均已有执行通过证据；完整界面与实际模型验收范围见当前验收审计（本地工作记录，不随仓库分发），不能由这些故障测试推断全平台 UI 已通过。

会话 sparse overrides 到物理模型请求的转换已收至 `nola_application::model_profile_resolution::{session_override_request,profile_with_session_contract}`，CLI 直接复用。各 native 调用仍在审查，不能将此源码抽取记作全端 sparse override 执行已完成。

新增 `resolve_effective_model_parameters` 只读 Application 查询的源码已接线，**尚待当前批编测与原生执行 caller 迁移**。输入严格区分 `current_conversation`（会话、配置 revision、预期 profile pin）与 `candidate_preview`（候选 profile pin、可选会话配置 pin）；同一一致读快照读取配置、模型目录与默认选择事实，复用上述纯解析函数。返回已验证的 profile、model key、原稀疏覆盖、独立展示字段和有效生成值；不返回 endpoint、credential reference 或完整 source_json。配置/profile 漂移或本机可核对的选择不一致明确拒绝。

`mlx` / `llama.cpp` 目前返回 `local_baseline_unavailable`，源 Host 执行返回 `source_host_required`；不能将本机默认值冒充这些来源的权威参数。源 Host 未经本机核验时 `verified_model_profile` / `model_key` 为 null。本批不替换其已有发送能力；这些来源的 baseline 及平台 caller 尚未完成，不能以查询存在宣称所有来源已统一。专家私有 preview 的新必需字段 `effective_parameters` 使用同一解析器，原 `model_declaration.parameters` 保持原事实。详细约束见有效参数设计（本地工作记录，不随仓库分发）。

### 公开思考输出与请求控制

`reasoning` 表示可请求的思考控制能力，`stream_reasoning` 表示已收到公开思考的增量投影能力，两者独立。宿主继续按实际能力构造请求；能力为 false 不允许擅自发送未声明控制参数，也不能据此丢弃供应商已返回的公开 reasoning / summary。原生 Apple、Android 与 CLI 事件适配器应先保留公开事实，再按 `stream_reasoning` 决定增量发布；禁用增量不等于清空已收到的终态思考。加密 continuation 只进入原 continuation 通道，不作为用户思考文本。视图是否展开思考属于展示状态，不能改变物理请求或抹掉 Core Part。


## 消息部件展示与控制来源

`JournalPartMetadata::Task` 与 `TaskControl` 是当前协议中不同的判别类型。前者只展示已保存的任务事实；后者才包含明确的 `supports_guidance` / `supports_cancellation`。Web journal 适配器须保留 `task_control`，不能改成 `task` 后仅凭同名字段恢复操作能力。按钮还须核原 journal 的 source epoch / conversation / Run / assistant 身份及当前 Host；canonical Task 摘要、历史记录与不匹配 owner 均不能授予操作。查看任务结果沿现有 Core task lineage 查询，不能把 `result_ref` 当可导航会话地址。

展示按公开 Part 原始序列进行；连续 Reasoning / Tool 可组成一段，Text（含空/未批准）、Surface、Error 与其它类型均是边界。模型请求只有实际 successor attempt 关系才表示重新请求；单独出现下一个模型名不构成证明。`sealed` 只表示执行结束，不等于需求验收成功。原错误 `code` / `display_key` / `recoverable` 保留为诊断；未知翻译使用命名失败状态，不把协议 key 当正文，也不从 recoverable 推导重试授权。

展开、焦点、输入法、阅读窗口快照属于客户端瞬时状态，按实际消息与 Part 身份保存，不改 Core Run。窗口宿主须在消息层，不能只把值移到父层却仍由会被重组销毁的 Part View 呈现 modal。离开消息 / 移除 Part / 迟到旧激活回调不得打开或清理新的阅读窗口；公开字段快照不授予读取 retained/private 原文或本机文件的权限。

Web 的 `MessageReadingScope` 只保存这些瞬时选择：owner 包含当前来源与消息身份，Part key 包含投影来源、journal owner、Part ID 与语义类型；字段 revision 更新不替换正在阅读的不可变快照。移除 Part、换 owner 或停止激活会失效原回调代际，重新出现同 ID 也不能复活旧 opener。内容标签隐藏时关闭 modal，但保留当前 owner 的显式展开选择；重新激活不自动重开。Svelte 选择值须保持确切快照对象身份（`$state.raw`），以同一身份进行 modal keyed 驻留及关闭 CAS。该 scope 不解析 A2UI、不维护业务状态或权限，不新增 Core/Host 接口。

Surface 公开投影沿可信 Store ordinal / origin / owner 与现有有界历史读取合并，不能在 checkpoint 后追加全部 Surface；真实空 Rebase 后已退休 Tool/Reasoning 保只读历史，预算延迟不升级为全文。该 producer 的当前 focused Store 回归已通过；各原生桥的新产物与真实 GUI 仍单独验收。专家表单后续 Run 的完整冻结见证尚未实现，不将当前专家只读展示写成动作已接通。当前验证和未完成边界见 `Doc/当前任务/UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md`。

## 精确专家回执读取与终态观察

Web / mobile Host 的既有 `Submission` 精确读取与 `Submissions` 列表读取共用终态观察：只有当前 receipt 为 executing 且已有明确 execution/context 时，精确查询现 `CanonicalRunTerminal`；尚未 Begin 或仍执行时无 seal，保原 receipt；读取错误明确失败。存在 seal 后由既有 Core `ObserveExecution` 重核原 submission / context / Run / originating_submission_id 并映射 receipt 终态。精确读取不启动 dispatch、不创建替代 Run、不以正文或传输 EOF 推断完成；列表原有调度职责不移到精确读。真实 Store 回归覆盖 claim-before-Begin、running、answered seal、幂等重复及 foreign owner，当前5项 Host suite 通过。

当前专家 manifest 的可读授权仍须完整可信 template-capture witness。缺该事实的测试/不匹配 Store 明确拒绝，不因已有文本或 sealed Run 补写见证，也不放松到无授权历史查询。客户端终态后需要的 canonical revision 必须取自当前公开读；不得用准入声明的序号或本地计数伪造编辑/分支见证。

Web `TimelineMessage.canonicalSnapshot` 将 canonical Message 与返回它的真实 source Host、会话及会话 revision 配对。历史读取安装同一响应的配对信息；编辑和专家来源均从该消息读取，不借用另一个分页窗口的 revision。终态后缺少该信息时，既有消息窗口 GET 可独立补读：只对完整已验证 seal 所声明的确切用户行安装 metadata，不替换可见 Parts、已加载范围、分页游标、journal 或 Run 状态。补读等待在飞历史请求，按 owner/激活票据、Run/seal、timeline version 作 CAS；切走、新 Run 或新历史读取使旧票据失效。缺失、不匹配或读取失败保留编辑不可用，不撤销终态、不重发模型。该客户端缓存附带读取来源，不新增协议或服务端状态机。
