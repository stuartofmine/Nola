> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 会话所有权整改：实施契约

> 后续架构决策：以 [会话独立配置与数据契约](../../软件架构设计/模块/04-存储与索引/CONVERSATION_AGGREGATE_CONTRACT_20261015.md) 为准。下文记录首批修复；内存草稿与切换时清空投影不是最终方案。后续实现必须按会话持久化并读取目标聚合，不维护全局“当前会话”数据镜像。

> 移动端后续收口：未提交草稿配置、跨窗口 CAS/删除墓碑、不可变附件、提交与恢复边界已继续实施，最新状态见 [iOS 草稿聚合](IOS_DRAFT_AGGREGATE_20261015.md) 和 [Android 草稿聚合](ANDROID_DRAFT_AGGREGATE_20261015.md)。本文下方“尚未完成”的草稿条目是前一批验收时的历史记录，不应当作当前进度。模型身份全面统一仍未完成，详见 [专项审计](../2026-10-19_当前任务归档/MOBILE_MODEL_IDENTITY_AUDIT_20261015.md)。

## 第二阶段：独立聚合与持久化（设计先行）

依照 [统一聚合契约](../../软件架构设计/模块/04-存储与索引/CONVERSATION_AGGREGATE_CONTRACT_20261015.md)，第一阶段清空投影/内存草稿不算完成。新增device-local ConversationDraftRepository：复用应用私有Application Support/files目录，按本地store scope及会话UUID隔离（远程Run仍引用本地canonical conversation身份，不以远程裸ID索引草稿）。文本manifest与安全app-owned附件资源分离；只存生成的UUID引用，不存外部原路径、URI授权、Task或连接凭据。新草稿有持久化UUID并在提交后迁移/退休；删除同时撤销内存操作并删除相应manifest，迟到回调不得重建。

配置由canonical ConversationRepository/LocalConversationStore提供：创建/显式选新模型时，将完整共享生成参数写到现有 `modelOverrides[modelKey]`；客户端特殊显示参数可放namespaced字段，但不得复制共享字段形成第二基线。加载已有会话只读保存的模型/参数，缺失模型或参数必须明确阻止执行，不能读前一个会话模型/当前全局参数。全局模型参数仅是创建或显式初始化新模型选择时的输入。移动端编辑会话参数不再写ModelParametersStore全局默认。

第二阶段验收增加：重建仓库读取A/B草稿/附件、无外部URI或凭据落盘、新草稿身份恢复与提交退休、配置默认改变后旧会话不变、已有会话缺配置不隐式补全。仍在整批实现后统一测试。

本轮范围是 [会话审计](../2026-10-19_当前任务归档/SESSION_OWNERSHIP_AUDIT_20261015.md) 确认的三个客户端问题；配置解析与 [LLM 场景契约](../2026-10-19_当前任务归档/LLM_SCENARIO_CONTRACT_AUDIT_20261015.md) 共用四层 owner，不增加配置优先级。先落此设计，再实施，最终由主代理统一测试。

## 模块与输入输出

| 模块 | 输入 | 输出/唯一写入对象 |
| --- | --- | --- |
| iOS ConversationComposerDraftStore | 会话 UUID、新草稿身份、文本/附件编辑、picker begin/result、导入 token/result | 对应草稿；当前页面只是所选草稿投影 |
| iOS attachment operation | 草稿 owner + UUID operation + 捕获 parser connection | 仅该 owner 的附件/notice/importing；stale result 无副作用 |
| Android HostSettingsOwnership | conversationID、model selection generation、捕获 Config、刷新 operation sequence | 是否允许把 Host capability snapshot 提交到当前 connection/UI |
| Android empty conversation projection | 保留的 app settings/model + 默认权限 | 显式新 owner 空投影，不携带任何原会话上下文/缓存/知识/运行/参数 override |

iOS 仓库实例属于一个 ChatView 的本地账户/数据库作用域；会话 UUID 在该作用域中定位 canonical conversation。新草稿使用独立 UUID，不与所有 nil 会话混用。附件导入是本地 draft 操作，切换选中会话不取消 A 的正确结果；删除 A、销毁整个仓库、提交清理会使 operation token 失效。picker 在打开时捕获 owner，剪贴板导入在粘贴时捕获 owner。远程 parser connection 在操作开始冻结，不能在完成时改用当前 Host。

Android Host 请求使用明确 Config 发请求，不让 fetch 函数读取可变 provider.config。提交前必须同时满足 conversationID、model generation、operation sequence 与当前配置相同；校验必须覆盖成功、错误、信任配对 await 后、finally等副作用。Host切换/新会话/重新选择导致 generation 不符，旧请求不得覆盖新连接及目录。信任记录允许按原 Host 地址保存，但当前 provider 只能在有效 owner 下更新。

## 持久化边界

草稿使用device-local仓储，配置使用既有Core canonical settings。UI草稿不进入模型历史，凭据不进入草稿。iOS按数据库父目录、Android按app noBackup私有目录划分store scope；文本manifest与app-owned附件资源分开。后台串行磁盘队列保存不可变快照，删除排在此前写入之后；读取在队列barrier后恢复。附件单项损坏时保留文本和其他附件并提示，不把整个草稿吞掉。Android上下文未知使用detail=null，不把未知缓存描述成测量0%。

## 取消与删除

- UI切换：只切投影；iOS原草稿导入继续，Android旧选中Host目录请求不可写当前投影。
- picker取消：结束原picker token，不改别的草稿。
- 会话删除：删除草稿时同时撤销其 operation；迟到结果不能重新创建 owner。提交清理同样撤销原operation。
- 取消任务：仍需token检查，不能依赖协作取消作为唯一保护。
- view销毁：撤销导入操作并取消任务，避免旧view回调影响重建实例。

## 验收与实施顺序

1. 独立草稿仓库单测：A->B迟到导入、并行A/B、picker取消、重复结果、删除后迟到、新草稿迁移、提交后迟到。
2. Host owner纯单测：A->B、相同URL不同token/pin/identity、后发先回、generation变化、错误迟到；provider fetch使用显式冻结配置。
3. Android空投影单测：有缓存/知识/参数/授权/trace的旧会话创建新会话后全部归属字段清除，app目录/模型默认保留。
4. 静态审计调用点，防止绕过模块；补记录已修/未修。
5. 最后统一 iOS/Android 测试，由主代理协调，不在实现中反复重编译。

## 实施状态与静态审计

- iOS新增 `ConversationComposerDraftStore`：单一草稿源，owner+operation UUID 验证；移除ChatView文本/附件与map两套可变状态。picker在点击时（键盘收起延迟之前）捕获owner；取消/重复返回、删除、提交和view消失均不能把迟到结果写入其他owner。导入成功、失败、notice和loading都写同一个草稿。
- Android新增 `HostSettingsOwnership`：成功/错误/配对await后统一门禁；fetch API必须接收 frozen Config。`setModel` 在配置准备的各个await之后检查selection，阻止请求尚未发出时就污染共享provider；读取会话override后也补最终校验。
- Android新增 `ConversationPresentation` 空投影构造器：清除所有会话消息、上下文/缓存detail、知识、来源、参数override、执行后端、授权、Run/trace/task投影，同时保留应用目录/模型默认。新建以及不同会话加载均经过此边界。
- 同类检查补修iOS Host能力刷新：operation UUID防同key重试时旧defer清新loading；自动后端选择只可写发起请求的会话，不可写切换后同Host的另一会话。
- 同类检查补修Android恢复观察：Config构建中的clientID/pin读取可挂起，构建完再次验证currentWindow才设置remoteObservationConfiguration，防止旧load覆盖新owner。
- 静态检查：搜索旧无参fetch/旧iOS草稿镜像写法，调用者已迁移；`git diff --check`和Swift `-frontend -parse`语法检查通过。未执行类型检查、构建、模拟器或测试。
- 新增4个iOS草稿测试、2个Android Host门禁测试、1个Android空投影测试，待统一执行。

## 明确保留的边界/未覆盖项

本轮已实现草稿/附件跨进程恢复及原子写入；未修改Mac单执行容量策略、未实现全端阅读锚点持久化、未重写Core Run账本。已有会话model或完整参数缺失会阻止执行，不再用上一会话或当前全局模型兜底。新建/显式选择模型才可初始化默认值；共享参数仅modelOverrides，客户端显示选项为clientModelPresentation。stopSequences沿用canonical换行字符串，seed沿用tagged值，reasoningPreference沿用既有枚举。

参数保存采用metadata字段归属合并和expectedRevision；Android参数编辑保留打开时的参数基线，iOS限制编辑保留打开时的conversation/model/limits基线，先检查编辑基线再CAS提交，防止陈旧编辑覆盖其他窗口。首次模型初始化与模型身份保存仍为两个事务，存在中途失败只保存参数但未切换模型的情况；失败必须展示，不允许静默成功。此边界不宣称是全端配置原子事务完成。

尚未完成的边界：未发送新草稿的模型/参数选择目前仍是创建前UI状态，只有文本/安全附件持久化；重启后不能声称完整恢复“未创建会话配置”。同一会话跨窗口草稿没有冲突合并协议，最后提交快照获胜。草稿load/delete目前会等待后台写队列，重型附件密集写入时的切换延迟需要真实设备量测。附件资源GC未实现，不删除用户原文件。Core的modelSelectionID统一初始化与移动端selectedModelInfo仍需后续统一收口，避免双身份。

第二阶段新增测试：ConversationDraftRepositoryTests（Swift）与ConversationDraftRepositoryTest（JVM），覆盖重建、独立A/B、附件损坏文本保留、删除排在写后、pending身份及迟到picker。仅完成静态diff检查和Swift语法解析，尚未运行编译/JVM/XCTest/设备验收。统一验收时须覆盖这些新增测试及CanonicalConversationStoreEndToEndTests、ConversationGenerationOverrideTest。

统一验证阶段 Android 结果（2026-10-15）：`cd android && ./gradlew testDebugUnitTest` 成功；JUnit XML 汇总111 suites，418 tests，其中416通过、2 skipped、0 failures、0 errors。包含新增草稿仓储、Host owner、空投影和更新后的参数override测试。未运行实体设备测试；Android通过不替代iOS或真实跨窗口验收。

### Apple 统一验证结果（2026-10-15）

- macOS：草稿改为真正设备本地文件仓储后重跑 `NolaMac` scheme、`Test` configuration，定向 `MacConversationDraftRepositoryTests` 与 `MacChatWorkspaceBehaviorTests`，195项：194通过、1跳过、0失败。真实Core同步队列保存/删除前后不变回归通过。跳过项为需要opt-in打开真实系统文件夹面板的 `testNativeWorkspacePickerCancellation`，未覆盖正常签名Debug应用。最终Mac证据为 `/tmp/nola-mac-local-drafts.A5ZUHl/Results.xcresult`。
- iOS：通过 `scripts/run_client_tests.sh`，单worker，实际可用的iPhone 17 / iOS 27模拟器，定向 `ConversationComposerDraftStoreTests`、`ConversationDraftRepositoryTests`、`ModelServiceConfigurationInstallationTests`、`ChatManagerTests`：39项全部通过，无跳过，xcresult runtimeWarnings为空。
- 验证中修正iOS草稿文件保护option名称；纯附件DTO和无共享状态存储辅助类型显式nonisolated，日志调用显式await，新增隔离警告已消除。剩余构建日志仅既有NolaCore Copy Headers/脚本阶段警告。
- 两个切会话测试显式持久化各会话模型及完整参数，不再靠manager全局模型给空配置兜底；权限冷恢复测试断言改为Core现有的 `blocked` + `permission.denied` + 非空blocker证据，而非把拒绝后的模型正文误认为任务已完成。保留冻结provider、HTTP次数、日志正文和terminal owner校验。
- 本机证据：iOS `/tmp/nola-ios-owner-validation.iMHU6y/Results-3.xcresult`、`tests-3.log`；Mac `DerivedData/Nola-etfchqtmfngzxnfmkehixzbnbyhd/Logs/Test/Test-NolaMac-2026.10.15_3-00-59-+0800.xcresult`。前两轮iOS失败记录保留，最终结果以第三轮为准。
- 最后数据库作用域整改后，iOS 原四类增量运行40项：38通过，新增同目录不同数据库/同UUID草稿删除隔离测试通过；两个既有流式/并行 ChatManager 测试业务断言通过，但teardown删除临时目录出现 `NSCocoaErrorDomain 513 / EPERM`，该轮如实记为2失败。未改代码、未忽略清理错误，随后同两项定向重跑2/2通过。证据 `/tmp/nola-mac-local-drafts.A5ZUHl/iOSResults.xcresult`（原失败）、`iOSRetry.xcresult`（重跑成功）；不声称40项一次性通过。两份xcresult runtimeWarnings均为空，临时目录清理偶发性仍为测试环境边界。

这些结果覆盖真实SQLite与模拟器单测，不等于真实UI视觉/动画、跨窗口重启、实体设备picker或Web IndexedDB多标签验收。以上实现阶段“尚未编译”记录描述当时状态，以本节最新验证为准。

Android目录刷新和iOS Host目录是Host级信息，只有其会话后端选择等写入才要求ConversationKey。权限处理与运行取消必须另用RunKey，不复用本轮Host刷新门禁。配置解析继续依赖现有Core/LLM模块，不在UI新增参数优先级。

统一验证建议：iOS `scripts/run_client_tests.sh -only-testing NolaTests/ConversationComposerDraftStoreTests -only-testing NolaTests/ChatComposerPasteTests`（使用有效模拟器）；Android `./gradlew :app:testDebugUnitTest --tests '*HostSettingsOwnershipTest' --tests '*ConversationPresentationTest' --tests '*ConversationUiContractTest'`。再补实际picker/Host延迟请求切会话交互，不能用纯门禁测试代替完整网络/设备验收。
