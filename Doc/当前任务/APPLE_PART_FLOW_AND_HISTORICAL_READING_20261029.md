# Apple 消息原序与历史全文：当前源码批

状态：源码实施/主审阶段，**未编译、未运行新增回归**。上批 Mac 27/27 只证明新 Core/preview DTO，不证明本批顺序、专家 UI action 或历史全文已完成。

## 当前展示契约

Shared `AppleMessagePartFlow.segments` 仅把相邻 Reasoning/Tool 组成连续过程段。Text、UiSurface、Historical、Error、Task、Skill、ModelAttempt、Source、Citation、Attachment、Compaction 等各自保原锚点。没有时间排序、正文去重、跨段拼接。整条正文聚合只用于显式 Copy。

Mac journal 和无 journal 的当前 Core projection 都接真实 helper。后者由 repository 一次保留完整 typed Core Message，作为纯展示值显式排除 MacChatMessage 的 CodingKeys；没有从 UI UUID、正文或附件数组重建顺序。无 canonical message 的 optimistic/plain model-test 消息只显示其明确纯文本，不另尾追加 Surface。

展开选择存于完整 message owner 下的 stable Part ID，段重新分组/前插后按 Shared `isExpanded` 投影，显式 false 优先。仅该段实际 running Reasoning 决定 live 默认可见；同消息已完成的前段默认折叠。单 Reasoning 段只有一个过程 Disclosure，正文不再内套折叠。Tool/正文阅读把实际 Part 标为展开，terminal 不移除正在读的详情；父显式折叠优先。Run 错误保持消息/Run 原错误位置，不注入每个已完成段。

普通与专家共用 source-bound Surface 包装。专家使用现有 child WorkspaceModel 的完整 identity/ref/generation/Host callbacks，并有父视图 lifetime 与精确 child/conversation/message/Run 作用域校验。没有新 WorkspaceModel/Runtime，没有父工具权限代入 child。

## 专家 Surface action 尚未闭环

`MacWorkspaceModel.deliverUIInteraction` 当前本机 queued receipt 走 generic `send(preassignedSubmissionID:queuedUserCommandID:)`，远端走 `wakeRemoteUIQueue`。Room reply 投影仍只锚定 complete manifest 原 child Run；没有 UI action successor Run 的完整冻结模型/指令定义与原 parent 关系协议。这不是 context 正确即可解决的渲染问题。

本批临时只读 gate 使用 **Core 返回的原 ref/revision 真实 readOnly snapshot**，直接可见；严格完整 identity、revision、readOnly/deleted、nodes actions/bindings 为空。既不本地改 phase，也不清 actions 制造只读。普通 Surface 保现交互。专家 follow-up 明确显示未接通，并保留为当前任务 open 项；不得宣告完整动作闭环。

## 历史全文的确切缺口

现安全入口：`MacApplicationPort.readHistoricalPart(owner, expectedConversationRevision)` → Application/Store `read_historical_part`。Store `historical_part_projection.rs::finish_historical` 在同读快照以 `allows_product_text` 过滤 pending/rejected 文本、markdown 与 canonical preview；显示端只消费该结果，不能使用整条 historicalFallback。

现不充分入口：

- `readConversationTextRange(owner, offset)` 无 revision pin，不能冒充原 historical revision。
- `readConversationExportRange(owner, field, expectedRevision, offset)` 虽有 Store read snapshot/revision CAS，但有意返回 raw archive；未执行 product text disclosure。不能用它绕过 Review 渲染气泡。
- `SemanticValue::Deferred` 已有 `SemanticTokenLocator {source,start_bytes,end_bytes,token_kind}`。其 source 含 canonical owner/field 与 Versioned 或 Immutable pin。Store 的 10 MB markdown 回归目前只在内部用 `published_part_body::field_range` 验原 JSON token 尾；没有 public safe token range reader。
- Journal Historical metadata 的 `canonical_revision` 可能早于后续 conversation revision；旧 pin 必须明确 stale，不能 UI 取最新 revision 偷换。若要已保留的 immutable 源版本，Core 须验证既有 immutable field reference 是否仍可读；没有则明确 unavailable，不重建数据。

### 建议最小共享读用例（尚未实现）

拟 `ReadHistoricalPublicTextRange { owner, expected_conversation_revision, field_reference, cursor?, maximum_output_bytes<=32768 }`：

1. `field_reference` 必须是 Core HistoricalProjection 生产的当前 typed reference，包含 owner、当前 schema 允许的公开 text/markdown 字段、source pin；deferred 情况含 exact SemanticTokenLocator。Native 不选择任意 JSON 字段/切 raw archive。
2. 同一 Store consistent-read 内核 owner、conversation revision/immutable field pin、source digest/locator、actual public disclosure。pending/rejected 返回不可披露；stale/foreign/malformed 明确拒绝，不能降级 raw preview。
3. Core 分页读取并增量解码 JSON string token，返回 decoded UTF-8 正文范围，不能把带引号/反斜杠的原 JSON 字节交 Native 自解析。对于 content_text 的当前公开字段复用现 published body reader。
4. cursor 必须绑定完整 owner/source pin/locator/decoded位置及 JSON decoder 状态，由 Core 生产/验证；不得相信客户端自报 decoder state。编码页受源 token span 约束，不能每次为后页重扫/加载完整 10 MB。
5. 结果含 reference/pin、text、next cursor/end，以及已知时 total decoded bytes；Native 按该 exact reference/generation 展示，导航或 pin 变化取消旧读。

当前精确 historical preview 是过渡加载/预览，不是全文交付。需要上述共享安全读口后，才接本段位置的完整只读 reader；本 renderer 批不猜造 Native JSON 解码或未经 Review 的全文通路。

## 待验证源码

覆盖原 R/T/Surface/R/Text、所有非 R/T 锚点、多个真实思考不去重、Part 前插/分组合并后 false 优先、终态详情阅读保留；当前 Core→repository typed message 真实 producer 与 Codable 排除；source child/foreign owner/晚读结果拒绝；生产组件 NSWindow/AX Tool Details 点击；长文单 Part 预算与 Copy 全文；actual Store 历史 Review pending/rejected 不披露。最终主审后统一编测，失败必须保留。


## 本批源审入口（新增源码尚未编测）

- `Shared/Presentation/AppleMessagePartFlow.swift`：linear segments + stable Part choices 的 false 优先决策（决策由 root 统一）。
- `Shared/GenerativeUI/AppleUIReadOnlyReferencedSurfaceView.swift`：直接可见的真实 Core readonly ref，与纯 `AppleUIReadOnlyReferenceValidation`。
- `NolaMac/Features/Chat/MacJournalMessageTextView.swift`：`MacJournalPartFlow`、`MacJournalPresentationOwner.processChoices`、实际 `ForEach(segments)`、段级唯一 Disclosure、原 Tool/BodyState 读取、单 Part reader；删除旧重排器与 continuations 视觉拼接。
- `NolaMac/Models/MacMessageModels.swift` + `Services/MacCanonicalConversationRepository.swift`：ephemeral typed canonical Message 值及真实一次 producer；Codable 排除。
- `NolaMac/Features/Chat/MacMessageView.swift`：无 journal typed `MacCanonicalPartFlowView`，复用既有任务/来源/压缩/trace leaf；不再尾追加 Surface 或视觉聚合正文。
- `MacMessageSurfaceContext.swift`、`MacTranscriptView.swift`、`MacAgentRoomsView.swift`、`MacConversationCanvas.swift`、`MacWorkspaceModel.isLocalConversationSource`：普通/child 共用 exact callback/lifetime 包装、readonly gate、本机文件源核验；canonical 无 journal 不默认 local。
- `MacJournalHistoricalPartView.swift`：同 DB/owner/revision 的安全历史 projection 位于原锚点；完整 public pinned 字符串 range 仍是共享缺口，未伪接 raw export。

新增测试源码：`MacMessagePartFlowTests` 7 case、`MacMessageSurfaceContextTests` 5 case、`MacMessageProcessReadingUITests` 2 NSWindow/AX 生产组件 case；`MacHistoricalPartDetailTests.testActualStoreHistoricalPublicReaderNeverRevealsPendingOrRejectedTextFallback` 及 `.testActualStoreHistoricalTaskPreservesSealedAsDistinctFromCompleted` 共 2 actual Store case。原 Journal NSWindow/长文/Copy/metadata 测试迁移到当前生产分段器，actual Store repository producer 增 typed Message 等值断言；非空 continuations 已无生产来源的测试不再作为假 renderer 验收。所有新增/修改测试只标为源码待运行。


### 阅读窗口 residency 收口（源码未运行）

- `MacJournalTextPreview` 不再持有或呈现 sheet；immutable reference/cursor/part/field/presentation 放在消息级 `MacJournalPresentationOwner.readerSelection`。`MacMessageReaderPresenter` 以一个 sheet 在 journal 或 canonical message root 呈现，R/T 段首前插、分组重组、子节点 onDisappear 不清阅读窗口。真正 message owner 改变仍清旧 owner 选择。
- historical details 同样由消息 presenter 承载；Journal 显式把原 DB 的 readonly reader 送入 detail model，避免误读默认数据库。无 journal 的预算 deferred historical details 按 Core 实际 selection 的 owner/part/revision 放回该 Part 原锚点，删除 canonical 重复尾部入口。
- `testActualFullReaderSurvivesProcessLeaderPrependAndRegroup` 使用 actual Store Tool 长正文/完整游标，真实 NSWindow AX 打开 Tool/Technical details/full reader，然后脚本改变纯展示段首并断言同 sheet 实例、同读取 ID 和原 reference 仍在。纯展示前插不是合法 Core checkpoint rebase 的替代；Core 实际 rebase 回归由共享层另验。
- 当前 Core ActivityState 唯一 `.sealed` 已加入 Mac DTO，保持与 completed 不同；typed DTO 与 actual Store 历史 Task 两项回归待运行。
- 原有 message-level Copy/Deferred reader 宿主不在 process Part 子树内，不受段级 ForEach 重组影响。历史 public pinned 全文端口与专家 UI action continuation 仍 OPEN。

- 最终 callback fence：message presenter 捕获 sheet UUID 与 UI epoch，旧 dismiss 不得清新 selection，旧子 Part open 与迟到 disappear 不能越过 owner UI epoch。nojournal canonical root 的 residency identity 含实际 conversation/message、Store DB URL、Core source owner（remote Host/epoch），不含正文/revision；切 source/message 重建 UI state，正常 cursor/文本流不重建。新 pure callback 回归覆盖同 epoch 旧 dismiss 和 A→B→A 旧 open/dismiss/disappear 拒绝；此值只展示、不授权且不归档。


### 首轮原生结果与共享状态修复

Mac r35实际77项：73通过、4失败（终态整消息高度旧断言、actual Store sealed历史、两个新Tool/reader AX点击）。整app/testtarget已typecheck；不得记录为renderer全通过。原始日志/xcresult保留在 `/tmp/nola-mac-ui-validation-20261028/ordered-parts-35.*`。

- AX发现custom DisclosureGroup label上的ID没有落到可操作Button；已将可选ID明确接实际Button，Tool/Technical原AX真实点击断言保持。
- 完成态独立Text增加12px使整消息总高度不代表过程收起；改为真实process控件按当前locale/Chat表的AX Collapsed断言，保留原展开/收起实际高度及用户选择检查。
- `sealed`确为共享Core历史识别旧固定state集合缺口。现ToolPartState/StepStatus唯一实现下沉Domain，Engine重导出同当前类型；Store以对应schema的Domain ActivityState/ToolPartState/StepStatus验证，不改变wire、不把不同状态归并completed。Shared Rust首轮23/23通过（Domain2、semantic6、historical9、Engine6），原Mac实际Store sealed断言未改。Mac失败4与typedsealed相关1仍待最新archive实际复验，不能用这23项替代GUI结果。


r36复验5项中3通过（实际过程AX收起、actual Store sealed、typedsealed），2个新阅读GUI仍未通过。r37采用完整公开AX树遍历仍2失败，实际树/bitmap证明 Tool 对外是 `chat.journal.tool / AXDisclosureTriangle`，不是内部style Button；此前认为label吞ID后需把ID接Button的诊断不完整。无效 `toggleIdentifier` API已移除，Technical恢复真实DisclosureGroup外层ID；测试现严格用原ID+原生AXDisclosureTriangle press、检查实际展开/阅读状态，process自身仍精确AXButton。读取窗口/owner/reference保留断言与原超时不变。此两案等待新复验，不预记通过；失败树/图见 `/tmp/nola-mac-ui-validation-20261028/reader-ax-r37/`。


### 定向复验终态（保留原失败记录）

- Shared Core 当前enum修复23/23首轮通过；Mac r36实际5项中3通过（原interleaved、actual Store sealed、相关typedsealed），r40仅余2个真实NSWindow/AX通过，0失败/忽略、3.936秒。未重73项无关通过项，不把这几次结果写成单次最新77/77。
- 最终AX结构使用Tool/Technical精确语义ID的唯一AXGroup，在该受限子树内找唯一原生AXDisclosureTriangle；跳过其它语义容器和actionable控件，seen去环且预算未遍历完明确失败。真实press后的Tool/technical/reader状态均断言，reader保持精确AXLink、Close AXButton。没有直接调用View action/Binding。
- `.accessibilityElement(children:.contain)`使Tool/Technical的语义容器ID不再覆盖所有后代；原有控制与布局、业务状态不变，保留真正的原生disclosure语义。
- 原阅读窗口经Part段首前插/重组仍是同一actual NSWindow sheet、同selection UUID、同完整Core reference与PartID。实际sheet.contentView的AppKit bitmap、Tool详情live/terminal两张均作为keepAlways附件保存，已导出 `/tmp/nola-mac-ui-validation-20261028/reader-success-r40/`；结果 `/tmp/nola-mac-ui-validation-20261028/ordered-parts-40.xcresult`。
- 全部失败日志/树/bitmap继续保留，r40后没有新增生产源码或重复编测；历史安全pinned全文与专家UIaction continuation仍明确OPEN，不由本批reader测试冒称完成。
