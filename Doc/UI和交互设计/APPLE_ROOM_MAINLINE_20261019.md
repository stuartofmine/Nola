# Apple 会话主线实施记录

当前产品目标已改为[统一对话与可保存 Agent](MULTI_AGENT_NAVIGATION.md)。下文保留具体实现与验收事实，不再把独立 Room 入口、Room 草稿产品对象或成员独立会话导航作为目标；这些实现需要按新契约收敛。

本页记录 2026-10-19 本轮实现证据，不替代当前任务总账。代码静态解析通过不代表编译、功能、截图或真实模型验收通过。

## 已实施

- 水墨青山颜色改为两端语义资产：纸白、墨灰、山青；浅深色与高对比变体。iOS 页面/聊天移除装饰渐变，输入框采用不透明阅读面、明确焦点轮廓；Mac 发送按钮采用 onAccent 配对。
- 内部过程在流式、失败及完成时均默认折叠；手动展开选择不受状态变更覆盖。覆盖 iOS 规范过程与工具组、Mac trace 与 canonical journal 两条真实显示路径。Mac 回复失败不能因为已有内部失败 trace 而省略外层提示；iOS 规范终态有独立失败/取消反馈。授权卡保持独立。
- iOS 删除强制收件人菜单及选择收件人后切换普通聊天草稿的实现。Room 自己拥有草稿与提交；Mac 新增原生房间窗口入口、房间列表/创建/加入成员。
- 两端 @ 候选只在用户选择时绑定稳定 participant ID；邮箱、粘贴文本和普通 @名称不会建立路由身份。当前接口支持一个显式目标，未指定时提交空目标，客户端不选默认成员。
- 发送同步建立稳定 submission ID 与本地“正在发送”反馈；Core 回执前不声称开始执行。接收后按真实状态显示等待分派/等待开始。失败保留原提交供同 ID 重试；晚到 ACK 仅清理对应草稿版本。
- 草稿与待提交票据使用当前数据库中的独立 Room product setting，CAS 防止多窗口静默覆盖；输入 200ms 合并、每个 owner 串行写。未知格式/错误 owner 明确报错，既有数据不迁移、不删除。提交前持久记录票据，重启恢复时同 ID 核对，不生成新 ID 重放。
- 切房间读取带 epoch；回执按 room ID 回填，不依赖当时选中项。现有成员 Context 规范投影、权限归属和事件顺序保留。
- Apple 无 @ 路由新增 room-scope / room_routing 辅助账本：冻结候选/输入，最多一次物理请求，严格单字段 participant_id 校验，再由 Core CAS 选择。iOS 支持现有 OpenAI 本地适配及配对 Host 的辅助 JSON 通道；Mac 支持配置的 Chat Completions / Responses 端点。不支持的适配明确报错，不默认第一成员。
- 执行前先读取/持久化 Core native execution intent（原 execution ID、平台、上下文配置 revision、模型 profile/revision、parent、文本），再 claim。iOS 用独立 ChatManager owner、Mac 用独立 MacWorkspaceModel owner 执行，不切主窗口当前会话。重启先查原 Run；已存在只观察，不存在则严格核对原 intent 后按原 ID admission，不能重新捕获配置。reconcile_execution 由 Core 核对真实 Run seal 或可验证的配置失效后更新终态，不把流结束当成功。Mac 普通侧栏隐藏 agentRoomOwner 私有上下文，Room 提供回复预览与独立执行/授权/取消详情。
- 模型页新增当前 Core 连接/账户入口：list/save account/connection、无推理的 GET /models 发现、显式启用模型、CAS 断开。凭据置于设备安全存储，Core 仅 opaque ref；执行严格绑定 ready account、连接、端点和协议，不回落其它密钥。受管 profile 的选择/参数与模型目录分离，不重写账户来源。未实现的来源导入与 OAuth 不冒充已支持。

## 尚未验收与接口待收口

- 独立执行/辅助路由/模型账户已通过下方集中编译和目标单元测试，尚未经过真实模型验收。Mac 已将预览替换为跨成员按真实时间合并的完整 MacMessageView transcript，正文/附件/过程直接呈现，工具仍默认折叠；权限、追问、未知副作用决策和停止在房间内处理，诊断恢复仍可打开独立 owner 详情。重启后显示与手动滚动仍须运行验证。
- Apple 远程路径已接 `remote_host` intent：先 prepare/bind 完整请求，再由 Room 引用 paired identity + request digest，claim 后读取原请求发送。重启先观察 source receipt；已 admitted 只恢复观察，未 admitted 用同 execution ID、原 body/digest 重放，不重新生成 tool plan。iOS 复用既有准备/事件消费流程，Mac 独立 owner 冻结平台上下文及 model intent。来源 Host 与独立客户端 Core 的真实 HTTP 冷恢复已通过末节 r10 验收；尚未覆盖实际 Apple App 进程退出/断网后的 UI 投影，不能扩称整条 Apple 界面恢复已验证。
- 辅助请求失败保留清晰错误，不自动重新派发；更多 provider 适配仍待覆盖。待分派消息取消已接 Core cancel_pending，实际 Run 取消沿用 owner 的既有协议。
- 当前 @ 呈现为编辑器内邻接可移除标签；完整光标内 token、方向键候选与 IME 行为、多 @、附件/引用仍需完成，不能称 Telegram 全功能等价。
- 必须补真实浅深色/高对比截图、最大辅助字号、读屏/键盘、窄窗与流式滚动验收。未测本地反馈 p95、首 token、帧耗时及真实路由模型。
- 单元测试已更新/添加：默认折叠、可见终态、色对、mention 身份、草稿版本/ACK、submit 同 ID 重试；本轮实际执行的目标范围见下方集中验证记录，其余测试不推定通过。

## 设计依据

使用 apple-design 完整技能及 accessibility、layout、typography、color、designing-for-ios、designing-for-macos、dark-mode、motion、loading、feedback、generative-ai 引用。关键依据为 loading › Best practices 的“Show something as soon as possible.”、layout › Visual hierarchy 的渐进披露、color › Best practices 的浅深色/高对比、feedback › Best practices 的就近反馈。水墨色值与 Room 所有权是 Nola 产品决定，不是 Apple 规定。

色对计算见 DESIGN_SYSTEM；真正的对比度以最终背景/透明叠加/系统覆盖为准。未运行窗口级视觉验证。

## 集中验证记录（2026-10-19）

- Mac：使用 `scripts/lib/xcode_test_device_guard.sh` 的 `macos` guard，运行 `xcodebuild test -project Nola.xcodeproj -scheme NolaMac -configuration Test -destination 'platform=macOS,arch=arm64' -destination-timeout 10 -derivedDataPath .build/apple-room-validation-macos -clonedSourcePackagesDirPath .build/real-mac-xctest-derived-data/SourcePackages -disableAutomaticPackageResolution ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO`，only-testing 为 `MacAgentRoomWorkspaceTests`、`MacModelConnectionServiceTests`、`MacModelImportBridgeTests`。构建成功，12 项测试、0 失败，进程退出码 0。
- Mac 日志 `/tmp/nola-mac-room-validation-cached.log`；结果 `.build/apple-room-validation-macos/Logs/Test/Test-NolaMac-2026.10.19_12-12-08-+0800.xcresult`。首次未复用包缓存的调用停留在 MLX 子模块检出阶段，主动终止（143），不是测试失败；后续复用已存在包缓存完成验证。所有产物位于隔离 Test 目录，未覆盖签名 Debug 应用。
- iOS：先 `xcrun simctl list devices available` 确认可用 iPhone 18 Pro Max（`BAC347A0-62AF-414D-86B1-4F7B41F39336`，iOS 27.0），通过 `scripts/run_client_tests.sh` 使用明确 Simulator UDID、`PARALLEL_TEST_WORKERS=2`、`-configuration Test`，隔离 `.build/apple-room-validation-ios`，复用同一 SourcePackages 并禁止自动包解析。only-testing 为 `IOSRoomSubmissionWorkspaceTests`、`IOSAgentRoomWorkspaceTests`、`IOSAgentRoomServiceTests`、`IOSAgentRoomHistoryTests`、`IOSCloudCredentialOwnerTests`、`IOSApplicationCommandEffectTests`。第二轮构建和测试成功，xcresult summary 为 34 通过、0 失败、0 跳过，退出码 0。
- iOS 首轮编译退出 65：`ModelSelectionManager.swift` 将非受管模型的可选凭据赋给必填字符串；修正局部 `key: String?` 保留匿名模型语义，受管连接仍使用严格账户凭据解析、不回退。第二轮日志 `/tmp/nola-ios-room-validation-r2.log`；结果 `.build/apple-room-validation-ios/RoomValidation-r2.xcresult`。首次曾传入不存在的 `IOSModelImportBridgeTests` 过滤器，第二轮移除，不计入覆盖。测试运行器完成 clone 清理与 guard 释放。
- 未覆盖：真实远程 Host、模型付费调用、进程退出/断网后恢复 E2E、窗口级视觉与辅助功能。上述单元测试通过不代表这些运行场景已经验收。

## Room 恢复与取消断点复核

审读发现并修正两处客户端缺口：其一，claim、路由、观察、取消与重载回调此前直接覆盖本地 receipt；现在只接纳同 room/input 且更高 revision 的回执，迟到状态不能将已取消/终态回退。其二，远程 request 已 bound 而 Room 尚未 claim 时，取消此前只更新 Room；现在对同 execution ID 的 prepared/bound 远程请求调用既有撤销/源端取消协议。准备期间遭取消导致 CAS 拒绝时再次清理；重启读取 cancelled receipt 时也重试清理。不会将其他 context 或其他 submission 的远程请求取消。

新增 `MacRoomRecoveryBoundaryTests`、`IOSRoomRecoveryBoundaryTests`，各 6 项隔离回归：持久 intent 不重新捕获；已有 Run 只恢复；取消 claim 不执行且触发清理；准备期间 CAS 失败读取 cancelled 后清理；切到房间 B 后，房间 A 迟到/低 revision/错误 input 回执不退回状态或改选中房间；远程 intent 进入 source 路径而非设备 invocation 查询。

- Mac 使用前述 Test/guard/隔离 DerivedData 参数，only-testing 为 `MacRoomRecoveryBoundaryTests` 和 `MacAgentRoomWorkspaceTests`，16 项通过、0 失败，退出码 0。日志 `/tmp/nola-mac-room-boundaries.log`；结果 `.build/apple-room-validation-macos/Logs/Test/Test-NolaMac-2026.10.19_12-32-47-+0800.xcresult`。
- iOS 使用前述 Simulator UDID、Test/guard/2 workers 与既有隔离缓存，only-testing 为 `IOSRoomRecoveryBoundaryTests` 和 `IOSRoomSubmissionWorkspaceTests`，xcresult summary 为 14 项通过、0 失败、0 跳过，退出码 0。日志 `/tmp/nola-ios-room-boundaries.log`；结果 `.build/apple-room-validation-ios/RoomBoundaries.xcresult`。两个平台各自新增的 6 项断点回归全部通过；此次未新增真实网络或实体设备测试。
- 证据边界：以上使用内存命令适配器与固定回执，覆盖客户端编排、owner/revision 防护及清理触发；没有杀实际应用进程，也未访问真实 Host。源端 tombstone、真正的重复 HTTP admission 幂等、断网取消送达仍需单独传输/E2E 验收，不能由这些回归推定已通过。

## 隔离真实 Host 传输验收与发现

新增 `scripts/test_real_room_transport_recovery.py`：本地 Room Application Service 与来源 HTTP Host 使用两个全新临时数据库；只读授权 LAN 配置，并仅向 `192.168.31.18:8081` 发起模型推理。退出时清理测试自己的配置/凭据副本，保留数据库与无密钥证据；不读取用户历史，不操作实际 Apple 应用或实体设备。

首次有效断点运行 `/tmp/nola-real-room-transport-r3.log` 退出 1：prepared 取消后两端重启，来源 tombstone 不变、迟到原 POST 不创建 Run 的断言通过；但 claim 后、首次 HTTP admission 前重启 Host，原 bound 请求实际收到 `409 host_tool_plan_rejected`。根因为工具计划此前只保存在进程内，冻结请求的 reference 无法冷恢复；该轮没有完成真实模型回复或重复 admission 验收，不算通过。

修复采用来源主数据库的专用 `prepared_host_tool_plans` 表与可信本机 IPC typed 命令，网络 prepare 仅接受身份、不能提交 snapshot。写入在事务内完成容量检查与 insert-only；Host 返回前持久化完整快照。读取核对原 host/client/context/reference、原 TTL 与当前目录约束，不延长有效期、不重新捕获替换计划。generic product settings 不能影响此表。本机 IPC 是可信服务边界，不是对网络公开的任意命令入口。

当前 Store 格式版本由 81 改为 82；不新增迁移/双读，也不自动重建或改写旧格式用户数据库。所有验证使用新建隔离 fixture。修复后的实际执行结果另记，不能由源码静态检查推定恢复通过。

`cargo test -p nola_store prepared_host_tool_plan -- --nocapture` 已退出 0，新模块 3 项测试通过：冷重开保留原快照/TTL并拒绝错误 owner/摘要；generic setting 不能伪造或覆盖计划；insert-only、256 上限与到期回收事务。其余 integration 测试均被名称过滤，不计入通过项。日志 `/tmp/nola-prepared-host-plan-tests.log`。

`cargo build --offline -p nola_cli --bin nola -p nola_service --bin nola-service` 构建退出 0（日志 `/tmp/nola-room-transport-build.log`，仅链接器 unwind 大小警告）。随后真实脚本 r4 首次冷请求已经 HTTP 200、Run=1，但旧共享脚本误读 V1 的 `seal.status`；已将共享 helper 更新为唯一 V2 result/seal/manifest 结构及独立域摘要校验，无旧格式回退。r5 正确检出 `blocked/context.input_budget_exceeded`，不能算模型成功；脚本现读取授权模型 `/models` 的 `meta.n_ctx`（本次 262144），在冻结 body 中明确上下文及 256 输出预算。

实际命令：`NOLA_RUN_LIVE_ROOM_TRANSPORT_E2E=1 NOLA_APPLICATION_SERVICE_EXECUTABLE=$PWD/target/debug/nola-service python3 scripts/test_real_room_transport_recovery.py --binary $PWD/target/debug/nola --service $PWD/target/debug/nola-service --config /Users/gaojc/.nola/config.json`。r6 已退出 0，日志 `/tmp/nola-real-room-transport-r6.log`，证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-transport-9hnove5u/evidence.json`：

- prepared Room 取消后来源及客户端进程重启，原 POST 返回原 tombstone，来源 Run 数仍为 0。
- claim 后首次 HTTP 前重启，读取同 execution ID/原 bound body，真实 LAN 模型回答包含随机标记，V2 result/seal/manifest 摘要独立验证通过。
- 完成前沿确定后重复 HTTP，以及再次重启后的同请求，均仅返回 receipt；来源 Run 数始终为 1，持久 Attempt 记录摘要前沿不变。

证据边界：是实际来源 HTTP 与独立客户端 Room/Core 数据库，不是 Apple App 操作，不覆盖加密 mobile bridge 或 UI 回复投影；没有旁路统计 provider 的物理 HTTP 次数，只有真实 Run/Attempt 持久前沿不变证据。r6 尚不包含之后新增的“目录变化拒绝→Room failed”场景；该场景结果待下一轮单独记录。

复用同一已编译 CLI lib testbinary `target/debug/deps/nola_cli-bc729b9b5cf0d1fa` 定向执行：`host_tool_plan_catalog_tests --nocapture` 2/2 通过（网络客户端不能传入 snapshot/TTL/schema、MCP 元数据准备不建立连接）；`submission_receipt_tests --nocapture` 6/6 通过（原 admission 无需再读取计划、身份/摘要/观察边界等）。两次退出 0，日志分别为 `/tmp/nola-host-plan-cli-tests.log`、`/tmp/nola-submission-receipt-tests.log`；未据此宣称整个 CLI 测试集通过。

后续 Store 最终定向运行 `cargo test --offline -p nola_store --lib prepared_host_tool_plan -- --nocapture` 退出 0、3/3 通过，包含新增极大 expiry 与未知 snapshot 版本拒绝断言，日志 `/tmp/nola-prepared-host-plan-tests-final.log`。第二次 CLI/Service 增量构建退出 0，纳入 Host 对过期/目录变化的真实 source rejection 回执修复。r7 启动阶段遇到 Service 的 5 秒 readiness 超时（未建来源 DB），没有执行业务断言，不计通过；未改超时或重建用户数据。

r8 新隔离 fixture 再次通过冷恢复和重复提交，但新增目录变化场景退出 1：来源已真实 `rejected` 且 Run 总数仍为 1，客户端 remote intent 已为 `rejected`，Room 却仍 `executing`。定位到 `remote_host` reconcile 查询本机来源表，而远程回执接收未写该表；已提交 Core 修复，不能称失败路径验收完成。日志 `/tmp/nola-real-room-transport-r8.log`，客户端证据目录 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-transport-ceji0zh7`。

最终修复改用 `exact_bound_remote_submission` 核对完整 identity 与原 digest 后读取本机已经验收源端回执的 intent phase；未绑定的本地取消不能作为来源拒绝证据。定向 Store 回归 `rejection_or_cancellation_retires_bound_intent_without_fabricating_terminal_run` 1/1 通过、退出 0，日志 `/tmp/nola-room-rejection-store-tests.log`。最终 CLI/Service 构建 `/tmp/nola-room-transport-build-r3.log` 退出 0。

完整真实脚本 r10 退出 0，日志 `/tmp/nola-real-room-transport-r10.log`，证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-transport-twxwoc0i/evidence.json`：上述取消、首次请求前双端重启、原请求重复 POST/Host 再重启全部通过；新增目录变化后原 POST 409、来源真实 rejected、客户端接收并 reconcile 为 failed，同时总 Run 数仍为 1，也全部通过。r9 在任何业务动作前因测试 helper 清理模块导入路径退出，已修路径，未据此改产品或放宽断言。测试自己的凭据副本与 vault 已清理。

进一步核对实际 Mac cold replay 发现它没有普通发送的 submission ticket，因此不能依赖普通发送 catch 的来源观察。`runBoundRoomRequest` 现于失败后按捕获的 target/identity 观察来源，接收 cancelled/rejected 就发出对应回执事件，不重新 POST。新增 `MacRemoteCancellationReceiptTests.testColdRoomHTTPRejectionObservesCapturedSourceWithoutResubmitting` 用真实本地加密 HTTP 和 Core 来源回执验证该路径；目前 parse 已过，实际 Mac Test 结果见后续记录。iOS 既有 processMessage catch 已执行对应来源观察，本轮不重复实现。

最终 Mac 集中验收使用仓库 macos guard、`Test`、隔离 `.build/apple-room-validation-macos`，复用已有 SourcePackages，未轮询实体设备或覆盖签名 Debug。首轮因新增测试夹具误用 `MacApplicationPort` 的私有 execute 编译失败，改为既有 `MacApplicationServiceClient`；第二轮发现模型探测真实 pipe reader 的 Foundation 定长读取互等，明确终止本轮测试进程（退出 143），由账户模块改为有界 POSIX read，未放宽断言。第三轮 `xcodebuild test` 退出 0：workspace 10、模型连接 5、导入 bridge 4、Room 恢复边界 6、冷恢复加密 HTTP 拒绝 1，共 **26/26**。日志 `/tmp/nola-mac-final-room-models-r3.log`；结果 `.build/apple-room-validation-macos/Logs/Test/Test-NolaMac-2026.10.19_1-38-52-+0800.xcresult`。这验证本地加密 HTTP 拒绝回执按捕获 owner 接收且不重发，不等同真实 Apple App 进程重启/UI 自动化；最新 iOS 未重跑。请求中的 `MacTranscriptPoliciesTests` 并非实际测试类，未据此计入通过，实际 `MacTranscriptCompletionScrollTests` 另行复用产物补跑。

滚动补跑 `xcodebuild test-without-building ... -only-testing:NolaMacTests/MacTranscriptCompletionScrollTests` 退出 0：9 通过、0 失败、1 跳过。跳过的是需显式提供私有 SQLite 一致备份的 `testRecordedLongConversationProjectionAndBodyReads`，没有读取真实用户数据代替 fixture。日志 `/tmp/nola-mac-final-transcript.log`；结果 `.build/apple-room-validation-macos/Logs/Test/Test-NolaMac-2026.10.19_1-40-54-+0800.xcresult`。

### Apple 模型账户入口与来源 owner 边界

复核 `MODEL_CONNECTION_IMPORT.md` 与实际路由：Mac 原生账户页已有明确文件选择→API导入预览/确认、本机目录/取消、单请求费用确认探测/取消和能力覆盖；iOS 本机支持 API 连接/目录/选择，来源导入与探测通过已配对且身份核验的 Host 页面。Codex、Kimi 来源登录只在 Host Web `/models?tab=api`，管理 API 额外限制 loopback，现有 Mac 私有导入桥没有登录操作。不能把手机远程打开页面当作可远程登录，也不能把 Host 账户冒称属于 Mac 原生库。

本轮补 Mac 原生账户页的已配对 Host 管理入口：显示所选 Host owner 与地址，点击按捕获 configuration 验证 pinned host identity，再申请短时浏览器 handoff；配置在核验中改变不会切换目的 owner。字面 loopback Host 明确提供本机来源管理，其余 Host 入口只承诺远端 API 管理，并提示在 Host 电脑本机浏览器继续来源登录。未配对时明确指向 Server 设置，不启动来源程序或猜测服务器端口。iOS 原有按钮改为“Host API management”，增加来源登录/取消必须在 Host 电脑本地完成的交接说明。两端都明确登录不启用订阅推理，来源 token 不复制到 Apple 本机账户。

新增字面 loopback/远端/伪装域名/非HTTP与userinfo边界测试，Swift parse 已通过；此次新增入口尚未集中编译测试，不借用前面的旧 Test 结果。没有真实OAuth、来源程序、本地模型或其他供应商请求。原生同 owner 登录桥仍未实现；这里是明确的独立 Host 管理入口，不是该能力的替代声明。

迟到回调静态收口：Mac 身份核验及 handoff 后检查取消，返回前核对当前配对的 host ID、地址和 token 与捕获 owner 一致；变化即拒绝，不打开旧 ticket。两端离开账户页取消当前 operation，打开 URL 前再次检查 cancellation，避免关闭页面后迟到打开浏览器。iOS 继续复用捕获模型的 Host identity 校验，管理目标固定 `/models?tab=api`；这些取消不宣称撤销已签发的短时 ticket 或已发生的来源操作。

后续统一验证完成：Mac 相同隔离 Test/guard 参数，账户服务 6（含新 loopback/远端边界）+导入桥 4，共10/10、退出0；日志 `/tmp/nola-mac-source-handoff-validation.log`，xcresult `.build/apple-room-validation-macos/Logs/Test/Test-NolaMac-2026.10.19_2-12-01-+0800.xcresult`。iOS 相同 Simulator/Test/2 workers，`IOSCloudCredentialOwnerTests`、`ModelServiceConfigurationInstallationTests`、`ModelSelectionCanonicalEndToEndTests` 共30通过/0失败/0跳过（1参数化两次，设备31 runs）、退出0；日志 `/tmp/nola-ios-source-handoff-validation.log`，结果 `.build/apple-room-validation-ios/SourceHandoffValidation.xcresult`。这覆盖最新入口编译与账户/模型回归，不声称实际浏览器授权或来源登录已端到端完成。

Web 来源账户文案另接现有 locale/t：`sourceAccounts.ts` 覆盖简中、英、德、西、法、日、俄，SourceAccounts/Codex/Kimi/OpenCode组件的静态显示、状态和只读user_action使用翻译键；来源返回的动态provider prompts/授权说明保留原文。认证≠推理、停止Nola观察≠取消来源、OpenCode callback/取消限制均保留。不修改登录流程，OpenCode的owner切换/新建ID轮换修复保持。`npm run check` 0错误0警告，i18n及新增catalog测试9/9；日志 `/tmp/nola-source-accounts-i18n-check-final.log`、`/tmp/nola-source-accounts-i18n-tests-final.log`。本轮无真实登录、provider调用或本地模型访问。

### Room 键盘与授权可达性增量

完整页面补验设计：依据apple-design `layout.md › Adaptability`“Keep functionality the same as size classes change”与`feedback.md › Best practices`“Consider integrating status feedback into your interface”，本轮保留现有水墨token、字体和页面结构，增加`IOSRoomConversationView`的submission依赖注入入口。隔离fixture直接宿主生产页面，使用临时Core双房间/成员与真实输入控件，覆盖390/834宽×浅深色的@候选、授权卡与处理中状态；发送仅写receipt，不安装任何模型执行器。切房间验证实际输入绑定与草稿owner，而非仅断言源码。

完整页最终r5通过仓库guard、Test配置、单worker与隔离DerivedData运行`IOSRoomFullPageTests`、`IOSRoomComposerInteractionTests`、`IOSAgentRoomWorkspaceTests`，**13/13通过，exit0**。日志`/tmp/nola-ios-room-full-page-validation-r5.log`，结果`.build/apple-room-validation-ios/RoomFullPageValidation-r5.xcresult`，12张完整页附件导出`/tmp/nola-ios-room-full-page-r5/manifest.json`。真实输入控件覆盖方向键/Tab选择、不误发送、提交receipt、切房间草稿隔离；候选和授权状态均断言输入高度至少64pt且位于窗口内。授权卡本来是300pt高的垂直ScrollView，初始截图只显示允许；r5实际滚动到底，四种尺寸/外观的actions截图均目视确认拒绝及补充说明可见，未实测点击拒绝、未执行授权工具，不扩大为端到端授权验收。修复暗色允许按钮白字在浅绿底对比不足，复用onAccentColor（对应对比约8.56:1，浅色约6.22:1）。这是生产页面宿主fixture，不包含系统键盘全导航、VoiceOver、最大Dynamic Type、真实iPad窗口或真实模型。

运行记录保留失败边界：r1为测试编译错误；r2因Simulator宿主spawn失败未运行用例，经精确核对取消，退出143；r3实际12/13，测试持有切房间前旧UITextView引用导致断言失败，改为查找当前挂载输入，未更改owner逻辑。r3收尾另阻塞在本测试simctl diagnose（子PID22938/父22464），依据sample和命令路径停止该诊断后原进程退出65，未删除用户模拟器。r4与r5均真实13/13通过，r5额外补授权滚动证据。所有测试均无真实模型、OAuth或实体设备。

审计发现两端 @ 列表原来仅触摸/鼠标可选；iOS 授权卡还依赖最新 assistant 消息投影，授权先于消息到达时没有独立入口。依据 apple-design `keyboards.md › Best practices` 的“Respect standard keyboard shortcuts”及已读 accessibility 输入规则，新增 Room 局部原生多行输入桥：候选开放时方向键移动、Return/Tab确认、Escape仅收起列表不删草稿；普通Return仍换行，Command-Return显式发送；IME marked text期间不截取候选/发送命令。候选具有稳定ID、@姓名标签、公开职责hint及selected特征；跨Room重置焦点与候选状态，确认后留在输入框。

iOS复用既有`HostPermissionDecisionCard`到独立owner状态区（有界滚动），解除对assistant消息存在的依赖，不修改permission request/decision契约；原消息内不再重复放同一卡。Mac既有独立授权/问题/恢复控件增加owner读屏分组。新增Mac3项+iOS2项真实NSTextView/UITextView命令与IME测试，覆盖不误发送、普通换行、显式发送；目前parse/diffcheck过，等待共享Rust接口冻结后集中Test。未进行VoiceOver人工操作、Full Keyboard Access系统设置或真实授权执行，不能据源码声明全部可访问性通过。

集中运行已完成（SIWC v8冻结后、后续Usage Option变更前）：Mac使用既有macos guard、Test、隔离`apple-room-validation-macos`，执行`MacRoomComposerInteractionTests`、`MacAgentRoomWorkspaceTests`、`MacRoomRecoveryBoundaryTests`、`MacRoomAndConnectionVisualTests`，22/22通过、exit0；日志`/tmp/nola-mac-room-keyboard-validation.log`，结果`Test-NolaMac-2026.10.19_2-48-49-+0800.xcresult`。三项真实NSTextView用例验证候选命令、marked text保护/Command-Return及普通换行。窄窗640×650的长标题/新输入区浅深色截图已目视检查，未见裁切。

iOS再次以simctl确认可用模拟器，沿用上述具体UDID、隔离Test/2 workers/现有包缓存，过滤器`IOSRoomComposerInteractionTests`、`IOSRoomRecoveryBoundaryTests`、`IOSRoomSubmissionWorkspaceTests`、`IOSAgentRoomWorkspaceTests`、`IOSPreparedRunModelTests`、`IOSHostOutputAdmissionTests`。33/33通过、0跳过、exit0；日志`/tmp/nola-ios-room-keyboard-validation.log`，结果`.build/apple-room-validation-ios/RoomKeyboardValidation.xcresult`。两项真实UITextView用例实际建立markedText并执行UIKeyCommand动作；HTTP普通路线明确`siwc_account:null`，构造/恢复仅接受binding v8，不加旧版双读。Mac对应fixture同步v8。

本轮iOS timeline四图导出至`/tmp/nola-ios-room-keyboard-attachments/`，390/834宽×浅深色均已逐图查看，正文换行与成员身份可读；仍是timeline组件截图，不含输入区/键盘/授权卡视觉矩阵。控件测试验证命令回调，不等同跨整个SwiftUI页面的键盘导航、系统VoiceOver或真实授权端到端。全部使用隔离fixture，没有模型/真实OAuth/实体设备请求；构建队列已释放。

### 最新 iOS 隔离复验与渲染

Usage集中Test已于上述接线后真实执行：相同隔离Simulator/Test/2workers，新增`IOSProviderUsagePresenceTests`2项与`OpenAIProviderTests.testResponsesPartialUsagePreservesUnknownTotalAndExplicitZero`1项，以及Room恢复6/workspace10/PreparedRunModel4，共23/23通过、无跳过、exit0。日志`/tmp/nola-ios-usage-presence-validation.log`，结果`.build/apple-room-validation-ios/UsagePresenceValidation.xcresult`。真实URLProtocol SSE证明partial计数不补provider total，真实0保留；无模型请求。此结果发生在后续Core scope联合证据覆盖修复之前，不能借为该修复验收。

### Mac同owner ChatGPT授权入口与原生发送

依据apple-design `managing-accounts.md › Best practices`“Always identify the authentication method you offer”、`sheets.md › Best practices`“Provide an alternative to the Done button”及已读accessibility/layout/privacy，采用原生账户页独立section与单任务sheet，不引入新颜色/字体系统。屏幕任务是明确授权本机Nola，而不是导入Codex token或操作paired Host。正文/标题使用系统text styles、现有surface/token，间距8/16/24；取消/关闭始终可达，状态使用文字而不只颜色，不添加动画。

新`MacSiwcAccountBridge`仅启动相同runtime与当前application.databaseURL的`model-siwc-bridge`，64KiB JSONL有界读取、固定请求ID与owner/flow/account/revision校验。UI在明确授权确认后start，服务校验官方https URL，每flow仅显式打开一次系统浏览器；authorized仅表示验证完成，需用户再按Save才能finish CAS。取消/退出页面后代际失效，迟到authorized不自动保存；服务错误不透传私密内容、不盲重试。管理入口接当前status/models/refresh/sign_out契约：只读status、显式目录GET不启用模型、独立刷新、退出二次确认与revocation_unconfirmed提示。SIWC连接不进入普通API-key discover/probe，目录后复用既有configure选择模型。

新增真实双向pipe owner/URL/精确account-session签出绑定及ViewModel迟到授权测试。iOS保持显式Host管理，未伪造本机callback。Mac普通发送与恢复使用独立typed SIWC选择：冻结profile ID/revision，不读Keychain、不生成API key字段、不使用普通API fallback；Host从精确Core profile读取私有授权。冷恢复保留原descriptor revision，不重捕获当前值。当前`mac_launch.model_profile_revision`为必填nullable字段，普通API明确null，不保留旧协议双读。

集中Test发现并修复实际生产断点：Core当前SIWC provider为`nola-chatgpt`，原Mac只筛选`openai-compatible`导致模型不可选择；现仅接受provider与`siwc_responses`严格配对，旧API编辑器不混入SIWC。r3中21/22通过，新发送/原revision/不读API key断言已走过，唯一失败为测试用通用upsert改受管profile被Core正确拒绝；fixture改用真实connection disconnect后再验证冻结选择拒绝，未放宽受管写入边界。最后集中结果见下方补记，不能将这些fixture称为真实OAuth或推理验收。

最终r4使用macos guard、`xcodebuild test -scheme NolaMac -configuration Test -destination 'platform=macOS,arch=arm64'`、隔离`apple-room-validation-macos`与既有SourcePackages；六个suite为`MacSiwcAccountBridgeTests`、`MacSiwcSignInModelTests`、`MacSiwcProfileAdmissionTests`、`MacLocalAgentCredentialGateTests`、`MacLocalAgentResumeFrameTests`、`MacModelConnectionServiceTests`，**22/22通过、exit0**。日志`/tmp/nola-mac-siwc-native-validation-r4.log`，结果`.build/apple-room-validation-macos/Logs/Test/Test-NolaMac-2026.10.19_4-05-32-+0800.xcresult`。真实Core临时DB+真实pipe+注入传输验证原生账户状态、退出迟到回调、SIWC新发送不读API凭据、无fallback、top与descriptor冻结revision、恢复不重捕获、connection变更后拒绝执行。没有启动真实OAuth、LLM或实体设备；未声称原生账户页人工视觉/VoiceOver或真实服务推理已验收。

后续文案审计修复：新增46个SIWC授权、阶段、管理操作及安全错误文案进入实际默认`Localizable.xcstrings`，覆盖en/简中/繁中/de/es/fr/ja/ko/ru九语言。保留独立owner、授权不等于推理、取消不等于远端撤销、未知刷新不盲重试等说明。`node --test scripts/test_siwc_localizations.mjs`扫描三Swift文件及phase/入口，验证每个key九语言非空、无英文回退、插值占位符一致，1/1通过；`xcrun xcstringstool compile NolaMac/Localizable.xcstrings --output-directory <新临时目录>`实际编译成功。该catalog增量未重跑Mac全量构建，也未声称九语言人工排版已验收。

Usage存在性后续接线（当前待集中编译）：iOS的`LLMStreamUsage`→`AgentCoreTokenUsage`→Core payload六计数均保留可选值；Chat/Responses不再给缺失input/output补零、不再把input+output冒称provider total，原始reasoning保留。单请求payload `request_count=1`，预算lower bound交Core计算；TokenUsage已删旧cache reported字段，ContextUsageSnapshot既有numeric+reported估算投影不变。client snapshot不再把context occupancy当作provider usage。新增解码/完整payload两项与隔离URLProtocol Responses partial SSE一项回归，parse/diffcheck通过，尚未运行，不能借用上述旧版22/33结果。

再次通过 `xcrun simctl list devices available` 确认上述 iOS 27.0 Simulator UDID，使用 `scripts/run_client_tests.sh`、`PARALLEL_TEST_WORKERS=2`、`XCODE_OPTS='-configuration Test -clonedSourcePackagesDirPath /Users/gaojc/Desktop/Nola/.build/real-mac-xctest-derived-data/SourcePackages -disableAutomaticPackageResolution'`，`DERIVED_DATA=.build/apple-room-validation-ios`，`RESULT_BUNDLE=.build/apple-room-validation-ios/RoomFinalValidation.xcresult`。过滤器为 `IOSRoomRecoveryBoundaryTests`、`IOSRoomSubmissionWorkspaceTests`、`IOSAgentRoomWorkspaceTests`、`IOSAgentRoomServiceTests`、`IOSAgentRoomHistoryTests`、`IOSCloudCredentialOwnerTests`、`ModelServiceConfigurationInstallationTests`、`ModelSelectionCanonicalEndToEndTests`。进程退出 0，xcresult 总计 **59 通过、0 失败、0 跳过**；其中一项参数化测试运行两次，设备执行数为 60，不重复算作 60 个独立测试。日志 `/tmp/nola-ios-final-room-validation.log`。最新 iOS 目录 transport 已随 App 完整编译，但此组不包含其 HTTP 超限/重定向的独立 iOS 回归，不将 Mac 对应证据外推到 iOS。

新增 `IOSAgentRoomWorkspaceTests.testRoomTranscriptRendersCompactAndWideInBothAppearances` 用临时 Core DB 与固定双成员回复，在实际 `IOSAgentRoomTimeline`/`ChatBubble` 上渲染 390×844、834×1112 点的浅深色四图。附件保存在上述 xcresult，并导出至 `/tmp/nola-ios-room-final-attachments/`（manifest 对应文件名）；已逐图目视检查，中英文换行、长成员名及浅深色内容均可读，无正文横向裁切。依据 apple-design 的 `layout.md › Adaptability`（“Preview your app on multiple devices”）与 `dark-mode.md › Best practices`（“Ensure that your app looks good in both appearance modes”）选择尺寸/外观矩阵；这是组件宿主截图，未验证导航栏、输入键盘、最大 Dynamic Type、VoiceOver 或真实 iPad 系统窗口，不能宣称完整页面视觉无障碍验收。未新增产品功能，也未调用 192 服务、真实模型、其他供应商或实体设备。
