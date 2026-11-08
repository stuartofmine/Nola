> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 一个会话中的多个独立上下文 Agent

用户明确目标：同一会话可加入多个 Agent，每个 Agent 拥有独立上下文；移动端对话 UI 参考 Telegram。此前“Agent 拥有各自会话列表”的解释不再实施。

## 进度

### 新目标：IM 式主控分派（2026-10-19，设计已更新，代码未实施）

整体视觉依赖：[视觉系统与组件规范](../../UI和交互设计/DESIGN_SYSTEM.md) 已明确主题浅深色、对比度、间距、排版、组件状态及动效；尚未全端落地。后续先统一 token 与组件，再实施页面，不能把本次文档更新算作产品完成。

设计依据：[多 Agent 群聊 UI](../../UI和交互设计/MULTI_AGENT_NAVIGATION.md)。此前强制收件人及成员草稿切换不是新目标的验收结果。

- [ ] Room 级草稿、用户消息、持久分派/交付契约与幂等恢复。
- [ ] 无 @ 主控选择合适成员；有效 @ token 直接指定，不靠关键词路由。
- [ ] 移除收件人栏，采用行内 @ 候选与多目标明确交付。
- [ ] 提交即反馈；接收、分派、排队、执行、失联与终态准确区分。
- [ ] 工具过程默认折叠；授权、需回答和阻塞失败始终可见。
- [ ] 全链路与多端 UI 验收，旧测试不能替代新契约验收。

### 已实现基线与上一轮 UI 整改

**UI 验收重新打开（2026-10-19）：用户指出现有 UI 仍未达标。下列功能完成项不代表视觉质量通过。当前集中整改消息层级、空闲状态噪声、成员导航、单一收件人输入区及大字体布局；新版截图与交互验证另记，不沿用 r11 的功能结果作为 UI 完成结论。**

本轮整改：两端直接进入群聊、标题管理成员、单一收件人输入区、身份头像和姓名、空闲状态隐藏。Android 消息双侧排列并修复大字体成员操作挤压；iOS 完整已解码文本不再重复展示历史详情，部分/未知内容仍可查看原文。根据 apple-design 的层级、点击区域、字体缩放原则实现，不添加装饰性动画。

- Android：20 项定向 JVM 与 APK 构建通过，既有隔离 fixture 实际检查浅深色、1.5 字号、键盘和成员页。截图 `/tmp/nola-android-ui-r2-final-light.png`、`/tmp/nola-android-ui-r2-dark-large-keyboard.png`、`/tmp/nola-android-ui-r2-members-large-fixed.png`。未重新请求模型。全局底栏大字号英文分字仍为已知 UI 问题，未声明全 UI 达标。
- iOS：`RoomUI-refinement-20261019.xcresult` 消息投影契约通过，首次视觉测试因缺 fixture 跳过，不计通过；随后 `RoomUI-refinement-visual-20261019.xcresult` 实际键盘、收件人切换、草稿恢复用例通过（48.089 秒）。附件 `/tmp/nola-room-refinement-keyboard` 已检查。
- iOS 有历史截图首轮 `RoomUI-refinement-history-20261019.xcresult` 标题可点击断言失败，未放宽断言；修复标题内容点击区域及历史视口裁剪后重验。
- 修复后 `RoomUI-refinement-history-r2-20261019.xcresult` 有历史三次启动的交互断言通过（57.020 秒），主代理检查 `/tmp/nola-room-history-refined` 三张截图，标题与收件人可点击、历史不再穿过顶栏，辅助功能大字体实际生效。**名为 dark 的截图仍是浅色，启动参数未覆盖应用主题，不计为 iOS 深色视觉验收通过。** 消息投影契约共 22 项通过。本轮是明确范围内的群聊 UI 整改，不宣称所有平台及所有窗口已完成视觉验收。

- [x] 可复用 Agent 定义、身份校验和默认配置领域对象。
- [x] Agent 目录规范 Store 持久化、修改/归档 CAS；4 项事务/重开测试通过。
- [x] 修正架构为 Room → Participant → 独立 Context，不把共享时间线作为模型历史。
- [x] Room/Participant 契约、原子加入和创建 Context、持久化。
- [x] 查询、明确收件人、运行/授权/取消接入现有调用链；权限/取消归属经契约测试验证，不宣称真实工具副作用验收。
- [x] 移动端成员管理、消息来源、收件人及 Telegram 式时间线实现（视觉和交互验收单独记录）。
- [x] 集成回归、重启恢复、真实模型上下文隔离验收；最后 iOS r11 已通过，证据及边界见下文。

目录服务不是多 Agent 已可用。SessionRuntimeRegistry 必须以独立 Context 为 key，而非共享 Room ID。不做跨设备同步，不自动迁移用户数据。

## 剩余验收清单

- [x] Core 串行及双 Run 共存的真实本地模型隔离、冷重开映射。
- [x] iOS 基础 11 项单测；追加晚到完整窗口排序用例后 Workspace 7 项全部通过。
- [x] Android 46 项定向 JVM、3 项真实 JNI；最终 owner 补充后 scope 5 项及 journal 12 项重跑通过。真实历史、键盘、A/B 草稿及重开保留。
- [x] iOS 创建/加入/键盘/草稿 UI 用例实际执行并检查截图。`build/RoomUI-20261019-r3.xcresult` 退出码 0，用例 58.746 秒通过；四张附件导出至 `/tmp/nola-room-ios-ui-r3`，主代理检查键盘截图：顶栏、收件人和输入区可见，输入区紧邻键盘。早期跳过及 TextField 查询失败样本均不计通过。
- [x] 两端群聊界面真实发送与冷重开；Core/Android 真实双 Run 重叠，iOS 独立执行并发契约测试与真实 A/B 串行 UI。授权/取消归属经 Core/iOS/Android 定向测试验证。

不将 Core CLI 测试或客户端单测替代移动界面验收。CLI 独立进程各自自启服务的生命周期竞态作为已发现限制保留，长驻同一 Service 的群聊并发验收通过。

### 当前完成审计

| 要求 | 当前证据 | 结论 |
| --- | --- | --- |
| 同一 Room 加入多个 Agent，各有规范 Context | Core 房间事务测试、两端页面及真实数据库 | 已验证 |
| 每位成员独立模型输入，不把合并时间线喂给模型 | Core 真实九轮隔离/召回、Android 不同 Context 的实际输出 | 已验证 |
| 设置、草稿、权限、停止与恢复均跟随 Context | iOS r2 定向回归，Android JVM/JNI 与原故障 fixture | 已验证；不冒充物理设备工具授权实测 |
| 一个成员故障不阻塞其他成员 | Android 保留 A 原 pending、B 实际 sealed；iOS 原意图恢复隔离测试 | 已验证 |
| 成员退出保留历史，不能继续向退出成员新发消息 | Core admission 与 iOS/Android 回归、移动成员操作 | 已验证 |
| 移动群聊界面、收件人和键盘交互 | 两端截图与 iOS 草稿 UI 测试；两端真实发送，iOS r11 退出码 0 | 已验证 |
| 真实发送后冷重开仍保留双方回复 | Android 真实 UI；iOS r11 冷重开截图、助手 AX 与规范数据库双 sealed | 已验证 |

本目标功能与移动 UI 验收已完成。后文保留阶段失败记录，不代表最终状态仍失败；不将真实串行 UI 误称双端真实同时输出，不宣称暖切换 p95 或真实工具副作用指标通过。跨端同步不在本目标内，通用草稿恢复原文入口作为独立后续体验项保留。

## 当前实现与证据

`nola-domain/agent_room.rs` 定义共同房间与独立参与者；`nola-application/agent_room.rs` 在规范 Store 保留设置键 `agent_chat_room/<UUID>` 保存成员关系，每次加入用 Application 生成新 UUID 创建真实 canonical conversation。Context 设置中保存反向 `agentRoomOwner`，实际 admission 检查双向身份与 active；配置使用现有 revision CAS，room revision 只控制成员管理和展示排序。退出保留成员记录和上下文，只禁止新发送。

`ApplicationCommand::AgentRoom` 已接入统一命令分发：目录、定义配置、创建、读取、加入、退出。只读命令与持久化写入分类明确。加入使用现有命令事务内的 savepoint，失败回滚，不生成半个成员。通用设置写入拒绝房间保留键。

`cargo check -p nola_application` 通过；`cargo test -p nola_application --test agent_room_contexts` 4 通过，覆盖同房间不同 Agent/不同房间同 Agent 的 Context 隔离、成员及 revision 校验、退出保留历史、重开恢复、命令读写分类。不是 LLM 隔离验收，也尚未接移动端 UI。

下一步：房间目录查询和参与者投影；移动运行时接房间命令；发送必须在 admission 中校验成员而不是先 resolve 后无条件发送。现有单会话路径尚未收口，不可宣称完整权限隔离完成。

## 续轮进展

已补房间分页列表命令 `agent_room/list`；iOS `IOSAgentRoomService` 与 Android `AgentRoomRepository` 均使用现有 Application transport，未建客户端旁路存储。Android 定向单测及主代码编译通过（3 项适配测试）；iOS 新文件仅语法解析通过，XCTest 尚未执行。

Store 在 `begin_turn_with_successor` 的同一写事务里重查 `agentRoomOwner`、房间成员与 active 状态，阻止使用旧选择向已退出成员发送新一轮；此门禁单测通过。普通单人会话无成员绑定仍按现有路径处理，不是旧版本兼容。远程 replica/内部维护/恢复路径仍需逐项审计，不能把这一个入口视为全链路已完。

通用设置写入和删除均禁止 Agent/Room 保留键。房间列表分页测试已补，待本轮运行。移动端服务层已具备目录、创建、加入、退出和查询，但群聊页面、并行流式投影及实际发送调用尚未接入；这些是下一步主线，不可跳过。

## Android 展示所有权接入（2026-10-18）

新增 `AgentRoomPresentation`，只聚合展示、不生成模型输入。每行保留 participant、独立 context 和上下文状态；组合消息键避免不同 Context 的消息 ID 冲突。退出成员保留历史但撤销收件人选择；切换房间清空展示引用，并忽略旧房间的运行回调。低 revision 的房间快照不可覆盖新状态。

`ChatViewModel` 的现有 RunRegistry 发布回调已接入该展示容器；选择收件人复用 `loadConversation`，发送前强制检查已加载的 Context 等于明确选中的收件人，避免异步切换时发给上一个 Agent。仍未接 UI 入口，不代表用户已经可使用群聊。

验证：Android 主代码编译通过，`AgentRoomPresentationTest` 5 项通过，覆盖聚合身份、切换发送门禁、退出保留历史、旧回调隔离和混合 Context 拒绝。尚未完成：移动端页面/成员管理、全成员历史加载、iOS 展示接入和真实 LLM 隔离验收。下一轮优先完成可操作页面，不继续扩展无调用方的基础类型。

## Android 可操作入口（2026-10-18 续）

`ChatScreen` 已接 `AgentRoomsScreen`：创建房间、以当前具体模型和参数创建 Agent、加入/移除成员、汇总成员历史和实时消息、显式收件人、群聊内文本发送。复用 canonical Repository、RunRegistry、MessageBubble 和持久化草稿，不向模型提交合并时间线。草稿仍由原有提交 CAS 清除，不在点击发送时先清空。历史气泡不接收其他轮的 live thinking/tool events。

`openAgentRoom` 从各成员自己的投影窗口读取历史；已有运行优先保留运行快照。Android Kotlin 编译通过，但未跑设备 UI、真实模型。权限/模型控制暂通过所选成员完整聊天控制页打开；尚需收口群聊内授权与停止、分页、加载竞态和布局打磨。当前列表仅首批 50 房间；不能作为最终分页体验。iOS 尚未接可操作页面。以上是进行中的实现，不是完整交付。

## 三路集成与验收（2026-10-19）

前述“iOS 尚未接页面”是阶段记录：当前已新增 `IOSAgentRoomWorkspace` 和 `IOSAgentRoomsView`，接入原聊天页面、成员管理、独立 Context 的展示订阅与明确收件人。尚未选择收件人时，不展示之前普通会话的草稿、错误、恢复按钮，也不允许配置入口编辑该旧 Context。普通会话历史入口退出房间展示，不合并模型输入。

当前三路并行分工：Core 负责 admission 与真实模型隔离验收；Android 负责页面、退出成员运行检查和模拟器验证；iOS 负责后台运行恢复、分页、模型选择与展示测试。主代理负责集成、所有权边界和集中构建。不得把子代理正在执行的验收记录为通过。

主代理最新检查：`swiftc -frontend -parse Nola/UI/ChatView.swift` 通过；这只是语法检查。前次完整 iOS 构建在 Workspace 的复杂表达式类型检查处失败，仍需修复后重新构建并跑 XCTest。真实移动端渲染、并行输出、授权/取消、重启恢复和真实 LLM 隔离证据尚未全部收齐，目标保持进行中。

iOS 第二次集中构建：Workspace 类型检查已修复，Rust arm64 模拟器产物编译成功；Swift 因 `onScrollGeometryChange` 要求 iOS 18 而失败，未执行 XCTest，结果保存在 `build/RoomTests-20261019.xcresult`。已改为现有界面同类 PreferenceKey 几何测量，并补齐七语言成员操作文案（`plutil -lint` 全通过）。返回房间列表清空导航选择而不终止成员执行，防止私有 Context 脱离房间后显示为普通对话。等待各端所有权过滤补齐后统一重跑，不能将上述语法验证视为运行通过。

iOS 集中重跑已通过（02:12，`build/RoomTests-20261019-r2.xcresult`，退出码 0）：Workspace 6、Service 3、History 2，共 11 项。覆盖独立执行实例、明确收件人、退出保留历史、旧事件隔离、同时间戳规范顺序、调用修订参数、错误传播、只读所有权投影及冷启动/普通历史隔离。XCTest 创建的模拟器克隆已由脚本清理。此结果不是 UI 截图或键盘/并行流式交互验收；这些仍继续进行。Android 实际截图已暴露并修复同时间戳顺序问题，键盘布局问题正在复验。

## Core 真实模型隔离证据（2026-10-19）

`python3 scripts/test_real_agent_room_isolation.py --binary target/debug/nola --service target/debug/nola-service --config /Users/gaojc/.nola/config.json` 最终通过。脚本只读模型配置，在临时独立数据库经正常 CLI 注册模型，再经 Application Service IPC 创建同房间 A/B 与另房间 A；没有直接写入伪造消息或直接向模型发两个 HTTP 请求冒充产品链路。

实际服务为 `192.168.31.18:8081`。三个 Context 完成五次 CLI/Core 模型轮次：A、B 各接收独立随机 token；冷进程再次调用分别只召回自己的 token；另房间的同 Agent 回答 UNKNOWN。规范消息里无对方 token，实际工具调用数为零，Service 冷重开后的两个房间及成员映射完全相同。证据目录：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-isolation-rtc209ud`，fixture 配置与凭证已清除，测试数据库和事件保留。

此前样本曾遇到 Host effect lease 超时，保留失败，未统计为通过。验收脚本已纠正规范文本读取字段（assistant 为 `content_json.text`）及模型进度事件分类（`tool.progress` 本身不是实际工具调用）。最终样本通过全部断言。此证据只覆盖真实模型上下文隔离、冷进程召回和映射持久性；不宣称移动 UI、并行输出、授权/取消、cache 统计或性能完成。

发送语义统一为独立 Context 的身份、active 与现有 configuration revision CAS，不采用全房间 revision 锁；新增其他成员不会使 A 的发送失效。删除了无产品调用的独立 `resolve_recipient`。远程 prepare 与首次 bind 和本地 admission 共用 Store 成员门禁；已绑定请求可恢复/取消。通用删除/整会话替换及全量归档恢复保护仍被房间引用的 Context，包括退出成员，反向所有权丢失也不把它默认为普通会话。

最后代码定向验证：Application `agent_room_contexts` 5 项、`persistent_agent_directory` 4 项；Store `room_` 3 项、`conversation_deletion_e2e` 5 项、`archive_restore::tests` 4 项、`remote_submission_tests` 11 项、`remote_sync_apply_e2e` 9 项均通过（筛选集有重叠，不累计作独立测试数）。新增删除/归档用例断言拒绝后原历史、checkpoint、revision 和同步发布不被破坏。真实模型验收在此后删除/归档保护加固前已完成；该加固由上述定向事务测试验证。

## 同房间并发真实模型验收（2026-10-19）

上述脚本增加 `--concurrent` 后通过，证据目录为 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-isolation-muxo2jlr`。正常 Application IPC 创建的同房间两个 Context 在同一长驻 Application Service 上分别启动 CLI/Core Run；只读数据库轮询记录两个不同 Context 同时处于非 sealed 状态，实际 run ID/status 保存于 `concurrent-evidence.json`。两轮均独立 sealed，规范输出各包含自己的随机 token、不包含对方 token。随后两个全新 CLI 进程分别再次召回自己的 token，最后 Service 重开验证成员映射一致。共九轮真实模型调用（五轮隔离基线、两轮并发、两轮冷进程后续），不要求模型服务内部同时生成 token。

首个两个 CLI 各自自动启动/退出 Service 的样本失败：一方遇到数据库 exclusive ownership 被占而默认 socket 不存在，证据保留在 `nola-room-isolation-37zod9_e`。成功样本使用产品共享运行服务对应的显式长驻 Service；并未修复或掩盖 CLI 自动 Service 生命周期竞态，不将该失败样本计为通过。

权限和取消身份边界补证：重跑现成 `permission_projection_read_e2e` 测试二进制八项及 `cancellation_wrong_assistant_or_run_never_changes_checkpoint` 一项均通过，覆盖错误 owner/revision、前轮授权视图复用拒绝、错误 run/assistant 取消不改 checkpoint。此为已有事务测试证据，不是新编译或真实工具副作用的授权/取消验收。

## Android 真实群聊推理验收（2026-10-19）

仅使用现有 `emulator-5554` 的本任务隔离 fixture，未访问物理设备或用户数据库。模型凭证通过 Android `MemoryProfileDataStore`/Keystore 产品 API 保存，准备文件自动删除，未输出或提交凭证。实际从 MainActivity 群聊 `Android-Fresh` 向 A/B 分别发送无工具、副作用的文本请求；A 输出 `ANDROID-A-OK` 和 alpha 编号文本，切换 B 后得到 `ANDROID-B-OK`。

规范数据库证据 `/tmp/nola-android-success-evidence.db`：A Run `d3346171-5a3e-4e62-90d8-b2b2939544fe` / Context `B0611A9C-FDAF-4FB4-A9ED-F3135E9D257D` 起止 `1792349157247..1792349217529`；B Run `040f5ff6-7bf2-425f-80ff-b050170903aa` / Context `E59E8FC7-D88C-4554-AB3E-1177B8CD233A` 起止 `1792349192036..1792349196669`。两条均 sealed，B 全部 4.633 秒落在 A 的 60.282 秒区间内。`/tmp/nola-live-current.png` 同时显示 B 的已提交回复与 A 的 Responding；`/tmp/nola-live-complete.png` 是完成状态。此为 Android→Core→真实模型链路，不是独立 HTTP 模拟。

真实调用发现并修复 nullable JSON builder 根因：`value?.let { put(...) } ?: put(..., JsonNull)` 使用了 `put` 的旧值返回值，使配置版本、父消息、Attempt/Turn ID、权限域等被覆盖为空。已统一封装 nullable String/Long 字段，并以实际 `buildJsonObject` 测试非空原值及显式 null。另发现草稿在 Admitted 时已 CAS 退休，但普通 mutableMap 不通知 Compose，输入框可能等待后续事件才清空；改为可观察 map，保留 ACK 与新编辑竞态的 CAS 保护。

失败记录保留 `/tmp/nola-android-failed-runs-evidence.db`，模拟器内完整旧目录 `no_backup.failed-room-evidence`（包括 pending command），未删除或自动修复。导入 CLI 已运行 Context 后直接续跑触发 checkpoint 配置不匹配，未将此样本计为通过；CLI warm checkpoint 跨客户端执行配置同步不在此次 Android Room 验收范围，不增加兼容回退或自动转换。旧错误 Attempt 响应遗留的 pending 暴露全应用单槽问题，已按下述所有权隔离整改并用原失败 fixture 验证 B 不受阻；fresh 双 Run 通过不代表 A 的旧损坏意图成功恢复。

后续直接复制原失败 fixture 复验（原 failed 与 success 目录完整保留）：A 的 pending request `a2c3dfaf-666d-407a-ae1b-af89c41480e0` 在测试前后 envelope JSON 完全相等，A Run `2761ac73-ee5c-4f10-b256-d681fd9442c9` 仍 queued；在同一 MainActivity 群聊向 B 发送短请求，Run `3955e9c2-91c7-47d3-a73e-f32b894ea4d3` / Context `5BB1B25B-EEC7-4D1B-8787-C61E7DCA276C` 于 `1792350651446..1792350656555` 正常 sealed，回复 `ANDROID-B-INDEPENDENT`。证据 `/tmp/nola-android-pending-isolation-evidence.db`、`/tmp/nola-pending-a-b-live.png`；冷重开仍显示 B 的规范回复及 A 的独立 pending。未把 A 错误意图删除或当作成功恢复。

此轮补齐 journal snapshot open/release 的 `at.identity.conversation_id` 所有权及纯 journal 读取分类；房间初始历史和分页逐成员捕获错误，不因 A 失败跳过 B。Admitted 后 composer 立即清空已由 Compose snapshot observer 单测及上述实际发送确认。主代理另独立复核成功 DB 的 quick_check、不同 Context、双 sealed 与起止重叠，以及 A 无 B 标记、B 无 alpha，均通过。

最终 Android 验证：46 项定向 JVM 测试全部通过（12 journal、4 command scope、2 nullable JSON、2 draft 观察、26 房间/运行/model identity）；3 项真实 JNI instrumentation 通过，覆盖 A 未确认/B 独立写入、同 A 互锁、精确冷恢复与原有 native recovery。最新 arm64 JNI、Debug APK 与 AndroidTest APK 构建成功，`git diff --check -- android` 通过。最终冷重开截图 `/tmp/nola-pending-isolation-final.png` 将 A 主状态简化为本地化“等待确认”，原始异常保留在展开诊断。临时 Keystore 模型凭证已通过产品删除 API 移除，并只读验证存储项不存在；模型配置准备文件早已删除。模拟器测试数据库及失败证据保留，应用已停止，没有操作物理设备。

## 移动 pending command 所有权设计与实施（2026-10-19）

Android 最后只读复核：两份已保存成功/隔离 DB 的 `quick_check` 均为 `ok`，双 sealed 重叠时间和原 A queued/B sealed 结果与上文一致；最终截图保留简短 pending、明确 B 来源与重新选择成员入口。本轮未再启动模拟器或重跑模型。生产代码 diff 未发现测试 endpoint、测试提示或凭据注入；opt-in instrumentation fixture helper 仅接受显式 disposable fixture，凭据不内嵌代码。原先的删除与磁盘确认已完成，本轮未访问用户配置。

末次审计补齐 7 个远端 consumer command 的精确 owner path，避免 A pending 把 B 的远端同步写入当全局命令阻断；新增单测覆盖每个合法路径、异 owner 不冲突、同 owner 冲突与错位 identity 拒绝。小型 `ApplicationCommandScopeTest`（5）+ `ApplicationSubmissionJournalTest`（12）共 17 项通过，跳过 Rust 构建。此前 46 项完整定向 JVM 与 3 项 JNI 结果仍保留；该末次映射补丁已 Kotlin 编译/单测，未再打包 APK 或宣称远端真实端到端通过。

发现 iOS/Android transport journal 只有一个全应用 pending 槽。某 Context 的不确定写入会阻断无因果关系的另一个 Context；这不属于 Core Run 本身的互锁。iOS 计划保留现有 requestID→完整 envelope 的原子文件格式和可靠写入流程，不迁移、删除或重写未确认命令；允许有界的多个独立所有者（最多 64 条，总编码上限维持约 32MiB）。

Scope 由客户端闭合的 typed command tag 与明确 wire schema 字段映射产生：已审计的单 Conversation 写入取其规范 conversation ID；未知、跨域和全局命令保持 global。不通过自然语言、字符串前缀或递归查找某个字段推断所有权。异 Context 可保留各自 pending；同 Context 或任一 global pending 阻止新因果写入。原 fingerprint 重试必须使用原 envelope/requestID，终态 receipt 仅退休匹配 requestID 的条目。

iOS 实现已落盘：有界多条 journal、同 owner/global 屏障、原 request ID 碰撞拒绝、按 Context 查询 UnknownEffect 决策、逐条冷恢复（某条语义/传输失败保留该条并继续，journal 不可读或持久化失败仍明确拒绝）。所有五处 ChatManager 查询已传明确 Context，查询不再解析别人的 UnknownEffect 决策。Core 新增只读 `load_run_identity { run_id }`，返回规范 `{ run_id, conversation_id }`，缺失报 `run_not_found`；复用既有规范 runs 查询，无 schema 更改。`claim_committed_host_requests`/`heartbeat_effect_delivery` 的 run-only owner 用此只读查询解析为 Conversation，不能将 Run ID 当另一个独立 lane。iOS 在已锁定的内部直接传输该查询，避免递归 submit 导致 NSLock 死锁。

新增契约测试覆盖独立 Context 重开、同 owner/global 阻断、只退休匹配意图、未知 command 保守全局、已知 command 错误路径拒绝、run-only 与同 Context 的因果保护，以及 A 的无法恢复 UnknownEffect 不阻 B 冷恢复/新写入且保留 A 原 envelope。Swift 语法检查通过；完整 XCTest 尚待集中构建，不能把仅落盘记录为已通过。

启动恢复应逐独立 owner 尝试，某一条失败保留原意图且不阻断其他 owner 的恢复。UnknownEffect 决策查询必须显式指定 Context，不能把别人的 pending 决策显示/恢复到当前成员。短时间文件及 runtime 事务锁继续保证 load/send/save 原子顺序；长期未决状态不再形成全应用互斥。

实施审计发现仅含 run ID 的 host claim/heartbeat 命令也必须解析真实 Context。不能把 run ID 当 Context，也不能长期归入 global 而重新阻塞其他成员：由 Core 规范 runs 提供只读身份查询，客户端据此确定所有者，缺失身份明确失败。查询不修改写入协议或持久化数据，不建立第二份客户端所有权事实。Core `load_run_identity` 已通过定向测试：无 pending effect/lease 也返回规范 owner，缺失或非法 Run 明确拒绝。两端集成验收范围以各自实际结果为准，不据此宣称整体交付。

最终定向审计追加修正（等待 iOS r8 回归）：当前调用的六项规范只读观察（committed host requests、runtime effect schedule、host execution state、claimable memory jobs、memory worker plan、remote submission intent）原先未进入 iOS observation 分类，会被别的 Context pending 当作全局写入挡住；现已逐项对照 Core 契约补齐。远端 journal consumer 七项写入原先漏 scope，现按确切 record/header、page/through 或 seal identity 路径归属 Context，包括 usage、admissions、content stage/receive、terminal begin/stage/commit。新增分类、异 Context 放行及同 Context 阻断测试，并在真实 Runtime 契约用例中加入 A pending 时 B 的 committed-request/remote-intent 观察。snapshot open/release 仍是 Context 租约写入，未伪装成只读。此次只改分类与测试，Swift 语法及 diff 检查通过，未重跑大规模构建；此前 r2 通过不涵盖这些新断言。

## iOS 最终验收准备（2026-10-19）

当前状态补记：r3 至 r7 未完成真实发送，不计通过。r3 是静态框架被全局强制签名导致构建失败，已将模拟器签名改为仅覆盖 `CODE_SIGN_IDENTITY=-`、保留目标自己的签名开关。r4 至 r7 的录像证实系统“保存密码”浮层遮挡成员页面；配置保存本身已成功，但 App/SpringBoard 无可操作的密码按钮 AX，不能以重试选择器冒充修复。

r8 改用严格门控的 DEBUG 隔离 fixture 准备：仅 `-ui-testing`、显式 opt-in、UUID 专属目录内既存 `nola.db`、路径无软链接且与 runtime 一致时，精确匹配唯一模型并调用正式模型发布/Keychain API。不会直接写数据库或明文保存密钥；重启去掉准备开关和密钥环境。后续收件人选择、发送、助手已提交回复与冷重开仍由真实 UI 验证。`build/RoomFinal-20261019-r8.xcresult` 当前运行中，另包含最新 command effect/pending owner 回归，结果待确认。

r8 已结束（退出码 65）：最新 PendingJournal 10 项、CommandEffect 5 项全部通过；真实 UI 用例失败，未计通过。正常选择 A 并发送后，产品日志明确记录 `checkpoint session configuration mismatch: context window, memory policy, turn identity`；导入的 CLI 已运行 Context 不适用于 iOS 当前运行配置。保留数据库 `/tmp/nola-ios-room-evidence.6Ptypn/nola-room-real-ui-2A0BA90B-53B7-4416-93E6-72F8BB3A21DB/nola.db`，不迁移 checkpoint。下一轮经正常 UI 新建 Room/Context，旧 Isolation 仅检查历史。

该失败还暴露真实展示缺陷：非 Web provider 在提交失败后置为 Completed；普通聊天把错误写入本地 draft，但 Room 仅消费规范 journal，没有新规范消息时错误不可见。整改为每个 execution 独立的临时提交错误投影，不伪造已提交消息；开始重试只清理该 Context 的错误，旧 sealed 更新不能掩盖新失败。该项正在实施，待回归。

r9 已结束：Workspace 9 项全部通过，包括新增提交失败归属、旧完成状态不抹去失败、重试只清自身与取消不伪报错误。真实 UI 已经通过正常界面创建房间并向 A 发出请求；规范数据库 `nola-room-real-ui-3D4A456A-BA59-4AF5-9370-DA103D05A0F5/nola.db` 的 Context `F0542E23-6484-4158-A6BC-6B8859161BBA`、Run `8d17c0aa-f147-4bbf-b6b9-6736cf176770` 已 sealed，助手正文精确返回 `IOSROOM0_6FA7C3B2358845049BFAD14C3247F645`。主代理检查录像最终帧 `/tmp/nola-room-r9-final.png` 确认实际显示。

r9 测试失败原因是查询把 Markdown 容器的 identifier 和子 Text 的 label 视为同一个元素。已改为在 completed assistant bubble 内查 committed Markdown 的正文后代，不查询用户气泡、不放宽回复内容。r10 继续验证 B 与冷重开；r9 的 A 成功不替代完整用例通过。

另有通用草稿体验限制：未 admitted 的原输入保留在 durable submission ticket，但当前失败后没有显式“恢复原文”入口，切换成员不保证自动恢复。不会自动重发或覆盖新草稿；这不是 Context 串用或原文持久化丢失，作为后续独立体验项记录，不冒充已验证的恢复能力。

新增退出成员授权与取消隔离用例：两个 Context 使用相同 request ID、不同 nonce；A 退出后处理 A 的权限和停止 A，不能修改 B 的等待权限、运行及导航状态。此用例目前只完成语法检查，待集中 XCTest。

真实 UI 验收经产品模型配置页保存隔离测试凭证，通过群聊收件人向 A/B 发出不同随机标记请求，仅匹配助手已提交 Markdown（不匹配用户输入以避免假阳性）。随后终止并重开 App，从同一隔离数据库重新进入房间验证成员及两条回复。fixture 已只读检查 quick_check=ok，现存十条 Run 均 sealed；端点和模型必须精确匹配原 profile。新增验收尚未执行，不能把此前键盘/草稿 UI 通过算作真实推理通过。

集中验收已启动，结果路径 `build/RoomFinal-20261019-r1.xcresult`，包含 Room Workspace/Service、pending journal、command effect、模型配置、跨 Context 停止/权限以及真实 UI 用例。使用 arm64 iOS Simulator 和独立 fixture，凭证只经测试进程环境传入。当前构建尚未结束，结果仍待确认。

该轮随后因 Swift `isEmpty` 属性误作函数调用而构建失败，未执行测试；已修复。第二轮 `build/RoomFinal-20261019-r2.xcresult` 编译完成，44 项测试通过（参数化执行在设备统计中为 45 次），无跳过；唯一失败为真实 UI 测试在 API 配置后找不到群聊入口（第 46 行）。尚未进入真实发送，不计推理通过。正在区分配置表单/导航问题与测试选择器问题，保留该失败结果。

交叉审阅追加两项明确 scope：`begin_automation_assistant_turn` 取 `input.conversation_id`；快照租约 open/release 分别取 `at.identity.conversation_id` 与 `snapshot.at.identity.conversation_id`。租约仍是写入，不伪装成只读绕开因果约束。iOS 房间初始化读取只读投影窗口，不使用上述租约写入；异步正文快照读取与其分离。

### iOS 最终通过证据（2026-10-19，r11）

本段更新上述分轮过程记录中的“待确认/未执行”状态，不删除失败样本。`build/RoomFinal-20261019-r11.xcresult` 退出码 0；`IOSAgentRoomRealModelTests.testExistingHistoryAndRealRepliesFromTwoIndependentRecipients` 用时 95.572 秒通过。真实 UI 先查看旧 Isolation 历史，再通过正常界面创建新房间、加入原 a/b Agent、明确切换收件人，分别发出不同随机标记请求；两条真实助手回复完成后终止并冷启动 App，重新进入同一房间验证成员与两条回复。没有追加重启后模型调用。

保留的隔离规范数据库：`/tmp/nola-ios-room-evidence.6Ptypn/nola-room-real-ui-180A6CE8-17A1-4FD4-8B7E-BE3CD3AFF85E/nola.db`。只读 `PRAGMA quick_check` 返回 `ok`，包含旧十条与本轮新增两条 sealed Run。新增 A Context `24C96B54-7CCB-4C8A-B13E-EA88B9A2D49B` / Run `10287537-de51-43f1-94f1-8e7ebd9ff70d`；B Context `BC2F2199-16D7-43AB-8497-E8F952B95387` / Run `ced6cc96-74d2-4467-933d-fde99fd76d40`。主代理独立核验两条规范 sealed 回复，不以 UI 文本替代持久化证据。

冷重开截图 `/tmp/nola-room-r11-attachments/1F291FF6-B8BF-41CD-8AD7-167378AB7E78.png`；对应实际 AX 子树 `/tmp/nola-room-r11-attachments/E378455E-4864-4E68-A765-ABB212319C02.txt` 展示两个 completed assistant 标记。r9/r10 的 A 实际已成功，测试错误在于预设短段落也有 `markdown.committed.*` 标识：实际 AX 的 assistant bubble 下只有无标识正文叶子，短段落渲染走 phrases 分支。最终测试仅在 completed assistant bubble 后代的 label/value 精确核验标记，不查询用户输入；首次完成、失败及冷重开均保存限定助手子树，未修改产品渲染或以重试掩盖失败。

覆盖边界：本轮 iOS 是两次**串行**真实模型请求加冷重启，不宣称 iOS UI 双流重叠。真正 Run 重叠由上文 Core 与 Android 样本分别证明。iOS A/B 权限、退出成员取消、失败状态归属及重试隔离来自 XCTest 契约回归（r2 集中测试与 r9 Workspace 九项通过），不宣称本轮真实模型进行了有副作用工具授权/取消。r8 最新 command effect/pending owner 十五项通过仍独立有效。

凭证与数据边界：r11 使用 DEBUG + `-ui-testing` + 显式 opt-in 的 UUID 隔离数据库准备，经正式模型配置发布与 `IOSCloudCredentialOwner` 写入该 XCTest 模拟器的 Keychain；规范数据库仅保存绑定引用，未直接写明文密钥。冷启动移除准备开关和密钥环境，不重新发布凭证。r11 诊断标识实际克隆为 `4C1F24B0-909D-45FF-A987-3C918AD699DF`；收尾只读检查确认该克隆已不在 XCTestDevices 列表且其目录不存在，其模拟器 Keychain 随克隆清理。保留的 host 隔离证据数据库不受清理影响；未访问或改写用户默认数据库、设备或用户 Keychain。

最终定向审计未发现新的多 Agent 功能阻塞。前述通用草稿“显式恢复原文”入口限制仍如实保留，不扩大为本轮新目标；不据本轮证据宣称跨客户端 warm checkpoint 配置转换、性能/cache 指标或所有真实工具副作用已验收。
