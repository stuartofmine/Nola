# 专家模板捕获授权：Core 实施细则

状态：**共享 Core / 私有 Host 与四端接线已实现并审查；Core 事务与故障、Apple 定向编测、Android JVM，以及 Web / Mac 当前真实模型重发已取得通过证据。iOS 新管理入口与真实 managed 模板发送、Android JNI / 实际操作仍在验证。** 具体计数、失败和修复保留在[当前验收账](UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md)，不能据此宣称全原型完成。本方案不修改既有存储数据，不为缺失身份补默认值，不建立第二套模板正文 catalog。

本轮审查新增两项：专家 context 配置须先按完整当前 schema 校验，再抽取冻结 defaults，未知字段不能被静默丢弃；预览额外返回同一 revision 下精确所选模型的 `client_model_presentation`，供真实 Host 指导 producer 注册，避免丢失会话自定义指导。该字段不进入 AgentDefaults 可写集合。Android 数值 sparse override 的旧 reader 仍可能在基础字段不全时整组退回 profile；共享转换已收至 Core，但 native 执行接线尚未闭合，不能记为已修复或已验证。

## 1. 已确认的真实缺口

| 位置 | 当前行为 | 风险与所需修正 |
| --- | --- | --- |
| `core/nola-application/src/expert_message.rs::custom_template` | 按 `selectedPromptTemplateID` 读取 `prompt.templates.customTemplates` 正文，未核可信当前 actor / ReadContent。 | 手工输入 ID 或 picker 展示后撤权仍可进入完整专家 manifest；必须在同一准入事务捕获正文前由 Core 再授权。 |
| 同函数的 `None` 分支 | 无 setting 或找不到 ID 返回 None，并注释留给内建 Host typed Turn 捕获。 | missing custom 与合法 builtin 未被证明区分；必须 exact lookup 得到来源，缺失明确拒绝。 |
| `core/nola-application/src/expert_prompt_catalog.rs` | 现只读列表已用 Core `evaluate_resource_access` 筛 ReadContent，仅返回 id/name。 | 保留复用，但只控制列表。新准入必须重新读取当前 resource revision/digest/grants/time，不能携目录结果充授权。 |
| `Nola/Services/PromptTemplateService.swift::loadTemplates/resourceOwnedTemplate/canAccess` | 对持久 custom 记录加载时补本机 owner / revision 1 / 空 grants；canAccess 再以缺省 owner/revision 判权限。 | 这是现存数据自动认领，不是可信事实。只允许**显式新建**时写 owner；读取已有 custom 缺身份必须不可用，禁止补值或回写。新 Core 捕获不能复用这种归一化结果。 |
| `PromptTemplateService.getTemplate(for:)` | language prefixMatch 分支只按 `language.hasPrefix` 查找，缺少 `canAccess(.readContent)`。 | exact/prefix/默认各选择路径必须统一按当前正文权限过滤；不能由语言匹配分支绕过读取授权。 |
| `PromptTemplateService.getCurrentTemplate` | 显式 selected ID 缺失或不可读时退回首个可读模板。 | 显式选择必须 missing/denied 明确报错，不得静默换定义；只有真实未选择状态才适用明确默认策略。 |
| `Nola/LLM/PromptTemplates.swift::builtInTemplates` + `PromptTemplate.init` | 内建定义由当前 Swift 代码构造，但未显式传 id，默认每次 UUID()。 | 不能作为可持久选择的稳定 builtin 身份。应在现有 Swift 定义源赋当前固定精确 ID；不迁移旧随机 ID，不按名称/语言/ID 前缀猜测对应关系。 |
| `core/nola-store/src/expert_message.rs::record_child_send` | 已对 typed PromptTemplate body 与预期 definition exact 比较，冻结完整 instruction witness。 | 该事实证明“实际使用这份定义”，不能替代“首次捕获该资源时有正文权限”。两种见证都需保留各自边界。 |

## 2. 唯一生产边界与不变的 Domain

保留 `nola_domain::expert_message::AdmitExpertConversationMessage`、现 `ExpertMessageManifest`、`ExpertPromptDefinition`、`ExpertInstructionWitness` 的现行 wire/存储形状。客户端仍只提交完整 participant refs、现 Room/context、原稳定 submission/message IDs 与正文。`created_at_ms` 是消息事实，不能充当前授权时钟。

在 Application **外层可信 Host 执行入口**增加只供当前宿主构造的 `HostExpertTemplateCaptureContext`，至少含当前 `AgentResourcePrincipal`、真实 Host wall-clock、可信 request Host 身份及当前 Host 内建定义提供者。它不进入 Domain input，不是 renderer 可提交的“授权声明”。不增加可选 actor 并在缺失时默认为 owner 的旧路径。

建议生产接法：

1. 公共 UI 业务请求保持仅 `input`；对应 Host admission 方法接收并验证现 Host/client/会话来源后，才构造可信 context。
2. 专用 native C ABI/JNI 与 Apple application service 私有操作复用已有 `attestEditedResendHostPolicy` 的可信工厂边界模式；不是把任意 actor/time 追加到 `executeAgentCommand` 的公开 JSON。
3. Rust Application 调用 `admit_with_host_context(input, context)`。普通可反序列化裸 Admit 路径不得在缺 context 时执行 custom/builtin 捕获；所有当前 caller 同批切到 Host 方法，不保留裸成功旁路。
4. 若用 `#[serde(skip_deserializing)]` 的 Host-only command variant，仍需专用 Host 入口从可信事实构造它；不能让 renderer 通过一个任意 JSON 代理“创建被 skip 的 variant”。跳过反序列化不等于鉴权。
5. 不把本次执行的动态 `now` 混入客户端稳定 intent 的 request hash。沿现 command receipt/稳定 input 幂等协议：稳定业务 input 与可信调用主体绑定，当前时钟只在第一次新捕获执行时评估。若现 receipt 适配层只能 hash 整个序列化 command，需窄拆 Host execution context 与 receipt payload，不能为了绕冲突沿用用户时间或复用旧时钟。

已核当前实际入口：`ApplicationRuntime::execute_received_command(ExecuteReceivedCommand, ApplicationCommand)` 把稳定的 `command_json` 与真正执行的 typed command 分开传入（`command_receipt.rs`）；Store `protocol_journal::same_command` 对 `command_json` 逐值匹配。因此可窄复用该入口：私有 Host method 构造带可信 capture context 的 typed Admit，receipt 的 `command_json` 仍由 Core/Host 固定构造仅含稳定业务 input 的当前 intent。context 不通过公共 serde 解码、不混入 receipt 的动态时间；调用主体由已认证 Host/client 边界绑定。无需改整个 Runtime 或 Domain manifest。不能允许调用方任意拼一个与实际 typed input 不同的 command_json；新窄方法必须由同一 input 构造两者并逐值验证。

建议当前 Rust variant 将整个可信 Admit 标记 `skip_deserializing`，capture context 不序列化为 renderer 可调用字段；公共 HTTP/mobile UI 请求 DTO 仍是当前业务 input，经过专用 native/Host operation 才转 typed command。现 generic service execute 不接受这个内部 variant，必须同批更新上述 Host adapter；不保留无 context 的旧成功分支。

`ApplicationCommand` 的外层可信包装可以扩展而不改 Domain/manifest；**不能只加普通可反序列化 actor 字段就宣告可信**。Apple 外置 service 无法自行读取对应 App UserDefaults 时，应由已认证 native Host 工厂传入固定 principal，经专用私有操作绑定到此 native 连接；Web renderer、agent 文本、模型工具参数不得决定该值。

## 3. 各 Host principal 与时钟

| 当前 Host | 可信来源 | 禁止行为 |
| --- | --- | --- |
| CLI / Web server | `crate::resource_access::LOCAL_WEB_USER_ID`，当前为 `cli-local-user`；服务端真实当前时间与实际 Host identity。 | 不接受 HTTP body 的 actor/owner/time；不以该 principal 自动拥有 Apple 目录。 |
| iOS / macOS | 当前 native `AgentCoreResourceAccess.localPrincipal` 的 UserDefaults 稳定值，现 key `ios.resourceAccess.localPrincipalID`；native Host 读取。 | 不把目录 resource.owner 当 actor；不因两个 Apple 平台名称相同或共享 Core 就合并不同 UserDefaults principal。 |
| Android | native Host 从 canonical setting `assistant.resource_principal_id`（现 schema wrapper）读取；DataStore 仅显式创建当前安装 principal，不认领模板。 | 不从 Compose input/expert defaults 取 actor；不把缺 principal 与缺模板 owner 互相补成授权。 |

Host 选择或身份变化不会自动改 resource owner、grant principal 或旧 manifest。没有当前 actor 时拒绝新模板捕获；不得退回 `Anyone`、模板 owner 或其它平台 actor。host_id 是否存在沿真实 resource host binding，不能把目录中原 `host_id:null` 擅自改成当前 Host 或清除已有 Host 边界。

## 4. Core 同事务捕获算法

`expert_message::admit` 的完整集合事务仍是唯一原子边界；正常发送继续现 Room/context。不得在客户端逐专家先捕获正文或逐个准入成功子集。

1. 检查稳定 input、Host/会话来源与完整 selected refs。查询同 conversation/submission 的 durable prior，按第 6 节处理幂等；没有 prior 才进行新捕获。
2. Core 读取原 participant 冻结 defaults 与当前 `prompt.templates` 原始 Store setting；不要接收平台归一化后的 catalog，也不要复制整份旧 system/history/resources。
3. 选中值为空/缺省：沿当前默认定义策略，由可信 Host 的实际 typed fragment provider 给出定义；并非把 UI 显示的“默认”解释为一个新的 None wire。
4. 显式选择 ID：只做精确当前 catalog 成员查找和可信 builtin 提供者的精确成员查找，不做名称、语言、ID 前缀或关键词路由。custom/builtin 双方同 ID 为歧义错误；双方均无为 `expert_template_definition_missing`。坏 catalog、重复规范 UUID、非法 revision/body/owner/grants 为明确格式错误。
5. Custom：从**同一读取快照**构造 `AgentResourceIdentity {kind:PromptTemplate, resource_id, owner, host_id, revision:accessRevision, content_digest:sha256:<actual UTF8 body>}`。owner、正 revision 和现 schema 必须真实存在。 grants 仅来自当前原记录，actor/time/request_host_id 仅来自可信 Host context。
6. 调用现 `evaluate_resource_access(permission:ReadContent)`。只有 allowed 才把正文写入 expected definition。metadata-only、过期 grant、revision/digest/Host 不匹配、撤销、其它 principal 均拒绝；不授工具执行权，不把 readMetadata 当 ReadContent。
7. `accessRevision` 是资源授权版本；`prompt.templates` product setting revision 是 catalog/定义读取 CAS。不得把二者混为同一 revision。现 `ExpertPromptDefinition.definition_revision` 的 catalog 快照语义不偷偷改变；资源授权见证由第 5 节独立私有事实记录。
8. 冻结完整集合 definitions/model declarations，并与 parent user、全部 selected child reservation 一起提交。任意一个未授权则整组 rollback，不能保存已授权子集或先提交再异步弹权限。
9. 每个 child 实际 Begin 仍走现 typed PromptTemplate witness exact 比较。首次授权不会替代实际使用证明；实际 Host fragment 的 ID/body/digest 与冻结定义不同仍拒绝。

当前 policy evaluator 已处理 revision/digest/expiry/Host/principal 的精确规则，应直接复用，不再创建第二套授权布尔值/ACL 状态机。

## 5. 最小持久授权来源事实

为了在不改 manifest wire 的前提下证明“原冻结定义确实经过合法捕获”，需要一个 Core 私有、同事务的捕获来源事实。不能只留临时 `allowed=true`，也不能把整个旧 grant 集合放进新 Run。

已核现 `StoredCommandReceipt`：只有 client_id/request_id/command_json/result_json/received_at 等，当前 Admit command_json 没有资源授权决定、实际 resource owner/revision/digest；client_id 也不是 resource principal。它不能证明原正文已经授权，不能把收到请求的时间或“已提交”本身当来源证明。本方案因此采用窄 `ExpertTemplateCaptureRepository`（不是再造模板 catalog），按 `(parent_conversation_id, submission_id, participant_ref)` 存唯一 capture fact。仅 Core 准入事务可写：原 resource ID/owner/host/revision/digest、可信 capture actor/Host、真实 captured_at、decision reason/matched grant ID、definition payload digest、所属完整 manifest intent。builtin 对应存真实 provider/catalog identity 与 definition digest。**不存可复用授权令牌，不把原 grants 作为后续执行输入。**

必须同时满足：

- 事务失败时 source fact、manifest、parent、reservations 一起 rollback。
- source fact 只能证明该已捕获字节的历史来源，不能授予当前 resource read/edit/share 或任何工具权。
- 重发原 source 时按 exact source owner/entry/definition digest 验证该历史来源，再走现 fresh owner/Run/policy；不能凭 display_name、当前 Join 或已成功 child 子集补全。
- 现版本记录若缺必要捕获来源见证，明确 `source_template_capture_witness_missing`；不迁移、不回填假授权、不因当前用户恰是 owner 就追认旧捕获。

此来源 fact/port 尚未实现；它应加入现完整集合事务，并由现源快照准备路径精确读取。现通用 receipt 可以继续负责丢 ACK 幂等，但不能代替这一来源事实；不能把日志文本作为见证。

## 6. 幂等与源快照重发

| 操作 | 应采用的定义与授权语义 |
| --- | --- |
| 新普通专家消息、全新 submission | 重新读取当前资源/current grants/current Host time 并捕获，撤权或变版本可拒绝。 |
| 同稳定 intent 的已提交重放 / 丢 ACK 恢复 | 确认 input exact、合法当前 Host/会话访问与原完整 manifest/capture fact，返回**原已提交结果**；不能重读变更后的正文重写原 snapshot，也不能拿旧 grant 新捕获另一份定义。未提交事务按当前权限重新执行。 |
| 原始 snapshot 编辑重发 | 复用原 manifest 已合法捕获且实际 child 使用见证完整的**原始定义字节**，不是再次打开当前模板资源。资源后续改名/改正文/删除/撤权不静默替换旧定义。保留原专家和模型快照，新建空 canonical owner/Room/context/Run，所有工具/Host policy/权限重新建。 |
| 重发中需要当前资源、MCP、Skill、文件等 | 每次通过新 Run 当前资源/工具策略重新授权。源定义里提及路径、工具或其它模板不构成授权；不可复制旧 resources/grants/history/permission outcome。 |
| 当前 caller 换了 principal/Host | 不继承旧 grant，不自动合并主体。必须先满足现源会话/Host访问边界；不能借已知 submission ID 读取其它 source 的 retained manifest/body。 |

“原始定义可重发”是对合法 retained conversation instruction snapshot 的复用，不等于资源 grant 可复用。历史授权只解释当时为何能够捕获那份字节；当前 resource 的新读写仍看当前权限。这一策略应明确写入接口说明，不作为隐藏兼容回退。若未来产品要求撤权同时撤销所有已留存会话副本，需另一项显式数据撤销协议，不能由本批偷偷删除/改写用户历史。

## 7. 内建来源收敛

- 真正定义继续来自现 `PromptTemplates.builtInTemplates` / `BuiltInPromptTemplates.templates`，不在 Rust 或 Web 另复制一套正文。
- 当前内建项须在原定义源获得稳定精确 ID，Host 在真实资源提供者查找后交付 typed definition 与 provider identity；Renderer 不能提交自称 builtin 的任意 body。
- 已有 typed fragment `PromptTemplate` 和当前 `ExpertInstructionWitness` 仍负责实际使用证明；不能用 arbitrary System string 或解析历史 system 拼出内建定义。
- 不支持该 builtin 提供者的 Host 明确 unavailable。Web 当前只读 custom catalog 无 Swift builtin provider，不假造 builtin 列表，也不能因 custom 找不到而把任意 missing ID 送进实际 Turn。
- 旧随机 builtin ID 不做自动映射/按正文查找/双读，不自动重写已保存 defaults。当前记录无法证明 exact provider membership 就拒绝重新捕获；可展示具体错误并让用户显式选择当前有效定义。

## 8. 调用改动清单与实施顺序

1. Core：抽出当前 expert_prompt_catalog 的严格 custom resource 读取/identity构造/evaluator调用为共用纯服务；列表仍仅投影 id/name，新 capture 返回授权后的 exact body/来源事实。不要把列表去过滤结果当缓存授权。
2. Core Application：新增可信 admission context/private Host入口、幂等 context 与 receipt 边界；替换 custom_template 的无授权捕获与 None=builtin 混淆；同完整集合事务持久 capture 来源事实。
3. Host：CLI/Web fixed actor；Apple私有Host principal/provider；Android canonical principal。所有现 caller 一次切到新入口，关闭任意公开 Admit JSON 的无 context 成功路径。模板目录查询也必须由宿主注入当前 actor，不能让通用 renderer command 查询任意 owner 的目录。
4. Apple：现原始 custom 读取去自动认领/默认 revision；新建明确赋当前 actor，缺字段已有记录拒绝；现 Swift builtin 源稳定 ID 与可信 provider，不建平行 catalog。
5. 实际 Begin / prepare resend：沿现完整 typed instructions/model witness 与 fresh policy，不新增每端业务状态机；关联 exact capture provenance，仅原定义可复用，所有资源授权排空重建。
6. 所有源码静审后一次编译/测试；未完成前保留本文件“未实现”状态。

## 9. 必要真实回归与验收

- 真实 SQLite 全集合准入：owner通过；跨 principal 无 grant / metadata-only / expired / revision错 / digest错 / Host错均整组回滚，parent/child/context/config不残留；目录先读通过后撤权，再准入必须拒绝。
- Apple真实选择回归：exact language可读、prefix language不可读不能返回；显式已选模板被撤权/删除时getCurrentTemplate必须明确denied/missing，不能退首个其它模板；真正未选择的默认策略才可返回合法默认。三条选型路径与Core准入同一readContent边界，不能只让目录picker/capture通过而语言路径仍绕过。
- 自定义 ID 存在但无 ownership facts、malformed catalog、重复 UUID 大小写、missing custom/builtin 均明确拒绝，无任何数据修改或自动认领。
- 可信入口：Web/UI payload伪 actor/time/provider被拒绝；原public裸command不能绕context；Apple/Android当前安装主体不能自动读其它平台owner。template权限永不授工具。
- 幂等：首次已提交后撤权/改正文/丢 ACK，同 intent 返回原合法 manifest且不重复 parent/child；input变化冲突；首次未提交遇当前撤权拒绝；动态 Host clock 不导致同 intent receipt 冲突。
- 原定义重发：捕获并实际 Begin 后改变/删除当前模板，原 source digest/完整见证可重发原字节，fresh owner/context/Run、无旧 grants/resources；缺 capture/source/任一 child witness 整组拒绝。
- Builtin：真实 Swift provider按稳定 ID产生 typed fragment；未知随机 ID、伪builtin body、不支持provider Host都拒绝；不把 missing custom 判builtin。
- 四端真模型：实际 UI 创建/选择合法模板、专家发送、改当前模板后编辑重发，检查 canonical manifest/capture fact/child checkpoint全文一致与fresh policy；再用无权限模板验证不会先调用模型。不能只用picker截图或DTO helper单测宣告完成。

## 10. 主审补充契约：真实默认选择与 retained source 访问

下述为下一源码批约定，**未实现**，不得由当前目录过滤或 UI 测试结果代替。

### 默认选择必须在首次捕获前解析并冻结

现 Apple Host 在 context defaults 没有 `selectedPromptTemplateID` 时，可能注入全局已选项或语言模板；所以 `None` 不能代表“让实际 child 任意附一段 PromptTemplate”。可信 Host context 应提交与本次完整 participant 集合一一对应的解析结果，按 `(context_conversation_id, configuration_revision)` 绑定：

- 显式选择：真实选中 ID 必须和 Core 当前冻结 defaults 的显式 ID 一致，失效/无权不做语言回退。
- 默认选择：真实 Host 唯一 provider 按当前全局/语言规则返回**有效 ID**及默认来源事实。custom 不传正文或 owner/grants 作为授权，Core 在同一准入事务重读 catalog 并评价 ReadContent；builtin 才提供现唯一代码源的 exact definition/provider witness。
- 真正无附加模板：明确 `none` 解析结果，实际 child 的 PromptTemplate 集合必须为空；不是有任意可读模板就偷偷选第一个。

Core 校验这份解析计划覆盖全部已选 context、每项 revision/身份 exact 后，将**实际有效 ID**冻结到现 manifest entry defaults（与现模型继承解析成实际 modelSelectionID 同类），并保存原始来源事实。不会修改原 context 配置、目录默认或其权限。

实际 Begin 要核准入捕获的 **PromptTemplate 完整集合相等**，不只是“预期那个 ID 存在”：显式/默认 capture 为一份就不能另附未授权 custom；none 为零就不能附任意 PromptTemplate。Host 当前 System/Skill/UI policy 属现不同 typed kinds，不被这项模板校验错误清空。编辑重发只从原已捕获完整 definitions 重建，不重新读取当前 global/language/default。

### 编辑 prepare 和任何正文读取也需要可信 scope

`PrepareEditedExpertResend` 也须走可信 Host actor/owner 入口，**在 prior/idempotent 返回前**核来源访问，不能让其它 principal 仅靠已知 source ID/operation ID 重放 retained 正文。原默认/模板 capture grant 不是这次访问授权。

当前没有跨 resource principal 的 conversation-sharing ACL，最小安全规则为：认证的拥有源会话 Host + 当前 actor 等于原 capture actor + exact source/entry/definition provenance。不同平台 principal 不自动合并；不因另一个 principal 是当前模板 owner 就允许读原私有会话 snapshot。未来显式分享必须有自己的当前授权契约，不能在本批隐式引入。

同一合法主体访问 retained exact definition 时可复用原字节，不重开当前模板资源；因此即使目录改名/删改/撤销旧资源 grant，也不会复制该 grant 给新 Run。新资源/工具操作仍从 fresh policy 开始。这同时满足“原始定义可重发”和“资源授权不可沿用”。

完整 body 路径应统一此检查：normal Admit 的 committed replay、edited prepare（含同 intent replay）、full manifest/source read、prepared result、child instruction/Host material read。public renderer 若只需判定专家/普通或展示状态，给 metadata projection，不在无 scope 的只读响应先泄露 expected definition/instruction witness 正文。只把 prepare 变私有而保留一个任意 principal 可调用的正文 load 仍不闭合。
