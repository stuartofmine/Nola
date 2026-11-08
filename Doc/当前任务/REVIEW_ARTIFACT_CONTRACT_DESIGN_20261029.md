# A02 / A03：审阅门控与产物版本共享实施设计

状态：**只读审计与下一批实施建议，尚未实现、未编译、未测试**。2026-10-29。

本文件独立于当前验证总账；不把已通过的模板、消息重发或 Git 读取测试算作 A02/A03 完成。这里的新类型、命令、端口均为拟议接口，不能按名称当成现有 API 调用。

## 1. 原型要求和生产边界

依据 `docs/design/nola-ui-prototype/docs/UI-INTERACTION.md` 第 3 节、同目录 `DESIGN-SPEC.md`、根目录 `FEATURE-COVERAGE.csv` 的“审阅/产物”行，以及 `UI_PROTOTYPE_ACCEPTANCE_AUDIT_20261029.md` 的 A02/A03：

| 项目 | 必须落地的业务 | 不能冒充完成的现有行为 |
| --- | --- | --- |
| A02 | 精确文件/hunk；发现严重度；解决/重开；请求修改/通过；重要未解决项阻止通过；明确打开完整 diff | 读取 Git status/diff、模型输出一串 `high/medium` 文本、按钮本地改颜色 |
| A03 | 产物身份/类型/来源；保留旧版；编辑保存新版本；指定两版比较；预览/错误/重试 | 把同 URI 的两次出现按时间标 v1/v2；显示当前文件；把上传附件称为生成产物 |

原型使用本地模拟版本和审阅状态，不是业务事实源。桌面点击检查器条目只选局部详情，显式“在标签页打开”才打开内容标签；手机从现工作工具二级页进入。版本编辑沿用明确保存/取消的模态编辑器，不引入任意标签未保存草稿协议。查看和审批不运行 Git 提交、推送或任何模型请求。

## 2. 实际可复用原语

| 当前生产实现 | 可复用部分 | 实际不足 |
| --- | --- | --- |
| `cli/src/tools/workspace_review.rs::read_host_workspace_review` | 真实 Host Git 读取；staged/unstaged 分开；禁止 external diff/textconv；有输出/时间上限 | 返回即时 inventory/文本，没有可重复读取的快照 ID、完整版本见证、hunk 身份或 finding/decision 账本 |
| `cli/src/web_parts/conversation_configuration.rs::conversation_workspace_review_response` | 核本地来源、会话配置 revision、读取前后 owner；现 Web GET 路由 | 指定的是该 Host 配置的 `project_root`，不能当任意会话文件根；核配置未变不等于核 Git HEAD/index/worktree 未变 |
| `NolaMac/Services/MacWorkspaceGitService.swift` | 真实本机 snapshot/diff、提交/推送；现有确认入口必须保留 | Mac diff 用 `HEAD` 组合差异，Web 分 staged/unstaged；两套语义不同；没有共享审阅生命周期。不能静默删除现 Git 写能力来“统一” |
| `cli/src/tools/parts/editing.rs::diff_review_findings` | 既有启发式提示可继续作为建议来源 | 输出是格式化字符串，没有 durable finding ID/原 hunk digest/解决状态；不能解析 `high:` 文本充当事实或审批 |
| `core/nola-domain/src/host_effect.rs::HostArtifact`；`public_part_semantics.rs::ArtifactReference` | 已验证的 Host artifact URI/name/MIME/operation；只读展示引用 | 都没有 artifact/version ID、不可变正文、父版本或来源 Host 授权。URI 本身不授予读写 |
| `execution_evidence.rs::ResourceEffect`、`PostconditionCheck` | 真工具效果的 before/after version、对指定资源版本的检查 | 版本字符串并不保留文件旧正文；ProcessExit 不证明业务正确；未知版本不补猜 |
| `cli/src/tools/parts/contracts_execution.rs::with_file_artifact_version`、`checkpoints_review.rs` | 编辑锁内取得旧 SHA、真实提交后取得新状态；现有路径/取消/原子编辑机制 | 产物引用与 effects 目前没有可浏览的完整不可变版本链 |
| `core/nola-store-port/src/canonical_body.rs` / `content_upload.rs` | 不可变正文的分块、配额、SHA、相同块幂等；staging/seal 与正式发布分离 | `CanonicalBodyReference` 是现有 Message/Part 字段 owner，不能伪造 Part 来存产物；upload seal 本身不发布版本 |
| `tool_source.rs::ToolSourceBinding`、assistant journal content | 可精确追溯真实 Run/turn/part/tool source，并保留来源见证 | 不能把整个工具输出/截断预览当产物原始正文 |
| `core/nola-store-port/src/work_step_transaction.rs` / `core/nola-store/src/plan_work_step_port.rs` | 既有 work/step/plan CAS、完整操作身份和终态事务；队列/Run 不需要再建一套 | 当前没有 finding/review gate/artifact version 的验收关系 |
| `run_requirements.rs` / `requirement_control.rs` / `run_result.rs` | 资源 URI+版本+predicate、确切用户命令来源、证据与评估/终态验证 | 当前的“需求评估”不是人工审阅票；不能把新 UI 的 approved 布尔值直接接成 completed |
| `workspace_checkpoint_store.rs` 和 CLI checkpoint helpers | 真实 Git checkpoint 的持久引用，可保留工作区恢复能力 | checkpoint 不是逐产物版本，且包含 workspace 范围；禁止借版本比较自动触发 restore |
| Application receipts、`ConsistentReadRepository`、exact Host effect recovery | 同 request 重放、CAS、统一事务、未知外部效果处理 | 新增业务仍必须用这些机制；不添加四端队列/授权/重试状态机 |

优先复用现存物理 content-object 存储和分块实现。若其内部函数现在强绑定 Part owner，应抽一次内部通用 immutable-object writer，再让 transcript 与 artifact 各用自己的强类型 owner 端口；不在平台建立第二正文目录，也不把新产物塞进假消息。

## 3. 最小共享对象：先确定产物版本，再确定审阅对象

### 3.1 产物

拟议 `Artifact`：`artifact_id`、`conversation_id`、`source_host_id`、`source`、`title`、`mime_type`、`head_version_id`、`revision`。ID 由 Core 分配/持久，不能由 URL 或文件名推导。

拟议 `ArtifactVersion`：`version_id`、`artifact_id`、`parent_version_id?`、内容 SHA/字节数/不可变 body reference、encoding、原始来源见证、创建者及真实时间。来源为闭合 typed union：

- 已接受的 tool completion：精确 conversation/Run/assistant Part/tool source/effect/delivery identity；Host 在真实写入锁内提供同一 postimage 的正文与 SHA。
- 用户保存的新版本：确切原 version + `expected_artifact_revision` + 新正文上传句柄 + 人类编辑操作 ID。
- 显式“捕获当前文件”：可信源 Host 当前读取权限、规范资源身份、观察时刻与 SHA；这是**当前观察的新产物**，不伪称还原过去工具输出。

不可变 parent 指向真实已存版本，Core 检查同 artifact、无循环、无跨 Host 错绑。展示 v1/v2 只由提交成功的链计算；时间戳、数组下标和 URI 重复均不建立版本关系。

首批完整支持 UTF-8 文本/Markdown/code 的编辑和双版本比较。现图片/PDF/二进制仍保留真实预览/下载入口；没有通用二进制保留能力时，明确 `preview_only` / `editing_unsupported`，不把二进制转成虚构文本版本。此限制必须在验收和 UI 说明，不将“所有类型编辑”记完成。

### 3.2 审阅对象

拟议 `ReviewSubject` 只接受两种不可变对象：

1. `ArtifactVersion`：确切 artifact/version/content SHA；不得跟随 mutable head 自动换版。
2. `WorkspaceChangeSet`：源 Host/workspace 身份、明确 base（含无 HEAD 的初始仓库情况）、index tree、工作文件/新增/删除/二进制清单与 SHA、冻结 staged/unstaged 原文和 scope digest。

Git snapshot 由可信 Host 生产，读取前后核同 HEAD/index/工作文件 digest；发生漂移返回 `snapshot_changed`，不得把两次不同观察拼成快照。snapshot digest 只证明那批字节，既不证明授权也不证明测试通过。外部 diff/hook/textconv 继续禁用，输出截断必须显式标记 incomplete；不完整 snapshot 禁止“全量通过”。

Core 用统一纯解析器解析保留的统一 diff，得到 `file_id/hunk_id`、旧/新区间和 hunk body SHA。Host 仅采集 Git/OS，四端不各自解析 hunk 归属。锚点属于精确 subject digest；换版后不按相近行号自动迁移 finding。

## 4. A02 的唯一 finding / gate 事务

拟议 `Review` 聚合：`review_id`、conversation/source Host、不可变 subject、policy revision、aggregate revision、finding 集合、immutable decision 历史。

`Finding`：Core ID、subject/file/hunk anchor、severity（info/low/medium/high/critical）、说明、来源（精确模型/工具见证或可信人类操作）、`open/resolved`、当前 revision。解决/重开记录确切用户操作、理由和可选证据引用，不删除原 finding。

最小命令建议：

| 拟议入口 | 必需身份/前置 | 同事务结果 |
| --- | --- | --- |
| `CreateReview` | conversation、source snapshot/version、stable operation ID | 核 source owner/可读性，创建唯一 review，不执行模型 |
| `AddReviewFinding` | expected review revision、确切 anchor；model finding 是建议来源而非人类确认 | 核 anchor/digest，分配 stable finding ID，增加 review revision |
| `ResolveReviewFinding` / `ReopenReviewFinding` | expected review + finding revision、理由、当前可信用户身份 | CAS 同时更新 finding/review；历史决定保留 |
| `DecideReview(request_changes/approve)` | expected review revision、subject digest、policy revision、可信人类命令 | 核全部 finding 与检查；高/严重 open finding 或 incomplete subject 明确阻止 approve |
| `LoadReview` / `ListConversationReviews` | 强类型 owner、分页 | 一个 consistent-read snapshot 返回 subject/finding/有效 gate；不启动执行 |

blocking policy 首批明确固定 high/critical，不能客户端删 finding、降 severity 或从请求里传一个可随意降低的 threshold 绕过。若以后支持策略编辑，须独立可信命令/CAS/留痕，不能并入普通 approve。

新 finding / 重开 / subject 不再是当前产物 head 时，历史 approve 票仍存在，但 `effective_gate` 为 blocked / stale / needs_review。批准只说明用户审阅了**那一版**。新版本不自动继承审批；关闭面板不等于通过；请求修改不自动重跑模型、恢复 Task 或授权工具。

本批 gate 的最小业务含义是“是否允许记录有效人工通过”。把它绑定到 Goal/Task 终态，需再接现 requirement evidence 事务：引用确切 review decision + subject version + 用户命令事实，在 Store result transaction 核其仍有效。不能跳过 `run_requirements` 直接把 Run 写 completed，也不能把 Git commit/push 当 approve 的附带效果。A04 尚未实现的验收映射继续明确未完成。

## 5. A03 编辑、比较、外部写入和恢复

拟议命令/查询：

- `ListConversationArtifacts`、`LoadArtifactVersion`、`ReadArtifactVersionRange`：强类型 owner、独立分页/字节预算、返回 `has_more`；来源会话的已加载消息窗口不是完整产物清单。
- `BeginArtifactVersionEdit`：只返回确切 source version 和配额；平台保存本地编辑草稿，不修改原正文。
- 沿用现 staging 的分块/相同 offset 重放，增加 typed artifact edit owner；`CommitArtifactVersionEdit` 在 receipt 事务内核上传 seal/真实字节 SHA、base version/head/revision、source read/write 能力，再创建 immutable 新版并 CAS head。
- `CompareArtifactVersions`：只允许同 artifact 明确两版；Core 统一纯 diff 算法、结果预算和截断标志；只读，不调用模型、不自动读取 URI 当前内容。

“保存新版本”首先保存 Nola 管理的不可变版本，保留源文件；UI 必须说明这一点。可选“应用到工作区/导出”是另外的显式 Host 写操作：复用现 permission resource plan、tool execution receipt、文件写锁及 `before_version` CAS。文件已被外部编辑则 conflict，保留草稿，不强制覆盖。

不要把文件系统写入和 SQLite 版本 publication 宣称为一个原子事务。若提供导出/应用，必须沿已有 Host effect ledger 记录 operation/计划和 exact outcome；断网/崩溃后的未知 write 只 reconcile，不自动重放。先确认真实新正文和文件 SHA，再提交版本关联事实。UI 的“重试预览”只重读 immutable 版本；“重试保存”只重放同操作/上传 identity，不生成重复 v3。

旧 `ArtifactReference` 未保留正文/版本时继续按引用展示，明确“未保留版本”。不能后台回填版本链、假设 URI 当前内容等于原产物，或迁移历史 Store。用户显式捕获当前内容是新的事实，不是旧记录转换。

## 6. 权限与 Host 边界

- Renderer 不能传可自证的 actor、source Host、文件可读/可写布尔值、Git root 或完成证据；由当前可信 Host API 构造并经 Core 核 owning conversation/source binding。
- 查看本地留存版本不授源 URI 读/写；远程 `file:///...` 永远不按本机同路径打开。远程 source 没有 snapshot/version read port 时显示明确不可用，不能退本机读。
- 人工 approve 不是工具/资源授权。复用目前 exact permission/resource-plan 检查；若将来开放独立产物分享，需显式新增受管资源类型和访问策略，不能借用 PromptTemplate/Skill 的 kind 或默认 grant。
- 所有 mutating 命令复用 existing Application receipt：business operation ID 与 transport request ID 分离；commit/savepoint unknown 保留 pending，同 request 收确认；确定 rejected 后新 transport ID 可重新评估同业务请求。
- Store 读聚合用 `ConsistentReadRepository`；真实 mutation 核同一批 expected revision/subject digest。平台仅保存 transient editor/focus/scroll，不能自行合并 finding 或版本 head。

## 7. 准确的实施文件与去重顺序

以下“新增”均是实施建议，不表示文件/API 已存在：

| 层 | 建议修改/新增 | 要点 |
| --- | --- | --- |
| Domain | 新 `core/nola-domain/src/artifact_version.rs`、`review.rs`；复用 `execution_evidence.rs` / `run_requirements.rs` 引用 | 闭合版本/来源/锚点/decision 类型，纯 diff/门控，当前 schema 严拒未知字段 |
| Store ports | 新 `core/nola-store-port/src/artifact.rs`、`review.rs`；审 `canonical_body.rs` / `content_upload.rs` 内部对象复用 | typed owner 的版本发布、finding/decision CAS 和一致聚合；不暴露任意路径写入 |
| Store | 新 `artifact_store.rs`、`review_store.rs`；内容对象内部 helper；`schema_store.rs` / `lib.rs::STORE_SCHEMA_EPOCH` 由 root 统一决定 | immutable body/version/event、唯一 operation 索引、同 receipt rollback、source/read-policy 校验。不得自行加旧 schema 迁移或自动重建用户库 |
| Application | 新 `artifact.rs`、`review.rs`；root 统一接 `application_command.rs` / `repositories.rs` / effect 分类 | 公共只读/明确用户动作与私有 Host snapshot/material 登记分离；typed read-only；复用 receipt/recovery |
| Host 协议/工具 | `nola-host-protocol/src/tool_result.rs`；`cli/src/tools/workspace_review.rs`、`parts/contracts_execution.rs`、`parts/checkpoints_review.rs` | 可信 snapshot/postimage 捕获；精确 resource/effect/source 见证；共享算法不进平台 |
| Host transport | `core/nola-service/src/lib.rs`、`core/nola-mobile-bindings`、`cli/src/application_port.rs` 与 Web routes | 私有事实登记不可从 renderer serde 伪造；既有 authenticated Host 路径复用 |
| Web | `WorkspaceReview.svelte`、`ChatArtifacts.svelte`、`lib/workspaceReview.ts`、`pages/Chat.svelte` | 新共享投影替代 current-only 字符串/已加载附件聚合；真实 findings/版本 editor/compare；保现内容标签行为 |
| Mac | `MacInspectorWorkTools.swift`、`MacWorkspaceReviewView.swift`、`MacWorkspaceGitService.swift`、`MacApplicationPort.swift` | 审阅读取逐步改同 Host/Core snapshot；删除重复 finding/diff 语义；保现明确 Git commit/push 能力；artifact 不再仅 attachmentURLs |
| iOS | `IOSConversationWorkTools.swift`、`IOSConversationInspector.swift`、`ChatContextSheet.swift`、`IOSApplicationRuntime.swift` | 为 Review / Artifacts 加真实二级入口和 Shared DTO；resources 上传附件保持独立来源 |
| Android | `ConversationInspector.kt`、`ChatScreen.kt`、`ChatViewModel.kt`、`AndroidApplicationRuntime.kt` | 同共享投影/命令接线，只有 editor presentation state，不复制 reducer/版本链 |

建议三段落地：先共享 immutable artifact/snapshot + read ports；再 review finding/decision/CAS + artifact edit/compare；最后四端调用与真实验收。每段结束先根静审，全部必要源码闭合后统一构建。现 Git 操作在共享等价能力接通前保持可达，不能先删后以空壳占位交付。

## 8. 真实验收路径与失败注入

### Store / Host 必需回归

1. 用临时真实 Git 仓库生成 staged+unstaged+untracked、rename/delete、初始无 HEAD 和二进制样本；Host 捕获快照，变更前后任何一个 digest 漂移必须拒绝原操作。external diff/hook 配置不能执行。
2. 两窗口同 review revision 同时操作：一个 resolve 后另一个 stale approve 明确 conflict；高 finding open 阻止 approve；resolve 后 approve 成功；reopen 后有效 gate 立即失效，旧决定仍可查询。
3. 保存真实正文 v1→v2→v3，退出/重开 Core 后逐版原文/SHA/parent 不变。指定 v1/v3 比较与直接内容计算一致；重复 request、丢 ACK、同上传 offset 重放不增加版本。
4. stale base、跨 artifact parent、foreign Host URI、本地同名远程文件、错误 actor、未 seal body、正文 SHA 不符、截断快照都拒绝；原正文/草稿不变。
5. deferred-FK commit failure、savepoint rollback failure、Host 写已发生但回复丢失：保持 unknown/pending，原 operation 收确认；不直接 rejected、不自动重写文件。
6. artifact/ref、tool effect before/after SHA、ProcessExit、人工 approve 的语义分别断言，不能相互代替；只有未来真实验收映射进 requirement result transaction 后才测 completed gate。

### 四端真实操作

在新隔离 Store 和真实 Host 工作区，通过产品入口实际完成：打开准确 diff/hunk → 新建 high finding → 点击通过被阻止 → 解决 → 通过 → 重开后失效；打开真实生成/捕获的文本产物 → 保存新版 → 逐版打开 → 比较两版 → 切会话/返回仍定位正确版本 → 外部改文件后尝试应用得到冲突并保留草稿。

分别保留当前 Core snapshot、服务 receipt、源 Host 操作见证、Store 版本链和 UI 操作/截图。至少一个实际配置模型使用真实写工具生成产物，再走捕获/审阅/编辑流程，证明工具来源接线；不是要求所有 CAS 回归都付费调用模型。模拟 fixtures 只验控件和异常展示，不能证明真实版本 provenance 或审批门控。

当前结论仍为：**A02/A03 未完成**。既有真实 Git/diff、文件编辑、上传和 Core 证据机制足够作为基础；缺少的是共享 artifact/version/snapshot 与 finding/decision 聚合，以及其四端完整生产 caller，不能通过增加几个按钮关闭缺口。
