> **范围说明（2026-10-20）**：下文保留当时 Room 实现与验收范围，不是当前产品结构规范。现行目标为[统一对话与可保存 Agent](../../../design/shared/MULTI_AGENT_NAVIGATION.md)；Room 与独立 conversation 映射尚待代码收敛。原测试事实不因目标更新而扩大适用范围。

# Android、Web、CLI 会话与账户呈现

负责人：other_clients_ui。状态：主干已实施，集中编译/单测与真实隔离 HTTP/CLI 验收已执行；视觉与设备交互边界见文末。

## 设计边界

### 无真实模型的 Room 权限交互补验

使用 `test_real_room_permission_terminal.py --synthetic-provider` 的显式隔离模式：不读取用户配置或凭据，只启动 loopback 脚本化模型；Room、Core 工具准入、权限回调、临时目录真实写入与终端 PTY 均使用产品实现。模型只产生固定 schema 调用，不直接注入权限事件、不绕过权限规则。依据 apple-design 的 feedback/privacy 原则，审批必须独立可见，呈现真实目标/覆盖模式/字节摘要，数字确认前文件不得变化。

该模式只补“审批可到达、数字确认、原 executionID 继续执行、单份 footer”证据；写入后显式取消。不能据此宣称真实模型调用、canonical 完成评估、浏览器视觉或系统读屏通过。真实模型验收继续暂停。

本轮实际执行：统一 `cargo build -p nola_cli -p nola_service --bin nola --bin nola-service --offline` exit 0（55.17s，`/tmp/nola-synthetic-room-pty-build.log`）。Web `node --test src/lib/agentRooms.test.ts` 5/5（`/tmp/nola-room-ui-owner-tests.log`）。Synthetic PTY r3 **未通过**：已显示真实审批、按 `1` 不加 Enter 后临时文件成功写入，但按 Ctrl-C 后 30 秒未恢复 composer。日志 `/tmp/nola-room-synthetic-permission-r3.log`，隔离证据目录 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-permission-pty-e11udsnv`。同时发现多行安全预览被作为单行送入面板，导致边框错位。两项保持待修；不计为完整权限交互通过。

取消接线设计：Room CLI 入口沿用 `ExecutionCancellation` 的 Esc/Ctrl-C 语义与独立 stdin listener；审批期间由现有权限输入锁排他接管。未执行的 accepted/selected 使用 Core `cancel_pending`，路由辅助调用共用取消 context；已执行则通过冻结 source identity+request digest 调用现有精确取消接口，不按房间或会话广播。Host 202 仅表示请求已接受，继续观察原 SSE 并由 Core reconcile 读取真实 seal；断流或请求失败不表示取消成功。Web 无 CLI listener 时不改变后台行为。面板按原始安全预览的换行分行，再按终端显示宽度折行；不得隐藏目标或摘要。

修复后 r6 整体 exit 0：`/tmp/nola-room-synthetic-permission-r6.log`，证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-permission-pty-zsxhtie1/evidence.json`。已验证独立审批安全预览/完整边框、审批前原文件未改、数字 `1` 无 Enter 后精确写入、Ctrl-C 取消同 executionID 的真实 sealed Run、canonical `cancelled`、恢复 composer、单份 footer、隔离进程清理。r5 同样通过交互但 cleanup 的既有全局 ps 快照超时，整体 exit 1，保留失败日志，不将其冒充成功。清理逻辑现独立执行各资源 cleanup 并把失败写入 evidence；不降低 process-owner 核查门槛。

定向测试：权限输入组 4/4（含多行回归，`/tmp/nola-room-permission-input-tests.log`），Room permission owner 1/1（`/tmp/nola-room-cancel-owner-tests.log`）；CLI build exit 0，`/tmp/nola-room-cancel-build.log`。仍未验证真实模型审批、系统读屏、各端完整切换视觉；Web 5/5 仍是纯逻辑测试，Android 本轮只读核对授权卡独立于消息折叠，未新增设备/视觉通过结论。

Nola 的会话页服务于个人与多个 Agent 的连续交流：以纸白、墨灰、山青保持长内容可读，Agent 身份与会话所有权比装饰更重要。沿用 [设计系统](../../../design/shared/DESIGN_SYSTEM.md) 冻结的浅深色语义色；Android 通过 Material ColorScheme，Web 通过 CSS 自定义属性，CLI 尊重终端背景和 NO_COLOR。

| 角色 | 浅色 | 深色 |
|---|---|---|
| canvas | #F5F6F2 | #101715 |
| surface | #FFFFFC | #18211E |
| raised | #EBEFE9 | #24312B |
| text | #202B25 | #E5ECE6 |
| secondary | #58665D | #ACBAB0 |
| accent / onAccent | #356B52 / #FFFFFF | #9AC7AC / #10251A |

字体现有系统字体，Android 正文 17sp、次级 15sp、标签最小 12sp；Web 正文 16px，次级 14px；CLI 字体由终端控制。布局沿用 4/8/12/16/24 间距、10/14 圆角；Android 控件至少 48dp，Web 触控至少 44px。品牌特征仅来自安静的山青操作色与墨色正文，不添加纹理、山景或新装饰。

按 sRGB WCAG 公式计算：浅色正文/canvas 13.49:1、secondary/raised 5.20:1、onAccent/accent 6.22:1；深色正文/surface 13.70:1、secondary/raised 6.72:1、onAccent/accent 8.56:1。计算是色值证据，不代表真实屏幕可访问性已验收。

窄屏：`会话列表 → [Agent 身份 / 有序消息 / 状态 / @ 输入]`。
宽屏：`[会话列表] [Agent 身份 / 有序消息 / 状态 / @ 输入] [按需详情]`。
动效沿用短暂 disclosure，减少动态效果时归零；流式内容不逐次触发布局入场。

## 启动时已有实现与差距（非当前完成状态）

- Android 已有 AgentRoomPresentation、AgentRoomsScreen、独立 context 草稿及状态；仍强制选择收件人。用明确的 @ 候选选择绑定 participant ID，不能按自然语言猜收件人。
- Web 当前运行入口是 Svelte，旧模板不作为本轮界面实现目标。常规工具分组已默认折叠且授权/失败展开；保留稳定身份与展开选择。
- Android HostToolTimeline 随事件 ID 列表重置展开状态且活跃时默认展开，需要改为稳定分组的用户选择；等待授权及失败仍外显。
- CLI 已有会话独立 viewport、输入/footer 分离及残影回归，保留这些约束，不增加另一渲染器。

## 协议与状态

主控派发及账户操作只消费 Core/Host 的正式接口；不创建展示层路由规则。未指定收件人交主控；显式 @ 绑定已选候选 ID；实际执行者以接受回执/投影为准。发送中先于异步调用显示，accepted、dispatching、running、permission、completed、failed、cancelled 不互相冒充。

导入入口先发现并显示脱敏计划，再由用户选择执行；UI 不接收、缓存或回传来源秘密。账号、连接、目录、运行 profile 保留各自身份，不把 URL/API Key 表单称为账号登录。

## 审查依据与验收

apple-design 的 accessibility、layout、typography、color、feedback、loading、motion、dark-mode 与 cross-platform 原则用于对比度、即时反馈和渐进披露；不将 Apple 控件惯例搬到 Android/Web。`loading.md › Best practices` 要求立即呈现反馈；`layout.md › Visual hierarchy` 要求渐进披露；`color.md › Inclusive color` 要求状态不只靠颜色。

先完成实现与测试用例，最后由主代理统一运行。需覆盖：浅深色对比度、动态字体、键盘焦点、减少动态效果、授权常显、工具展开不被流事件重置、跨会话状态/草稿隔离、CLI resize 与中英文输入 footer 稳定。真实模型和性能数值尚未测量，不写已通过结论。

## 落地接口与静态整合

- Web `/api/agent-rooms` 只开放目录、房间、提交、投影和待派发取消；Host 后台辅助调用、Core 收件人选择、执行 claim、既有 canonical chat runner、真实 seal 观察形成单一路径。CLI `room send <room-id> <text> [--to <participant-id>]` 复用该执行路径。
- Android 的房间仍由本机 Core 拥有。Host 辅助调用 `scenario=room_routing` 接收 `{message,candidates:[{participant_id,name,agent_id,description}]}`，只返回候选 ID；本机以 `{kind:room,room_id}` 观察 auxiliary ledger，再 SelectRecipient、PrepareExecution、ClaimExecution 并在独立 context 执行。不向 Host 复制本机 room。
- Web 连接与账户只投影脱敏事实；导入 plan/commit 分离且来源或凭据授权选项变化使预览失效。目录发现不发送付费 completion。反向网关有独立 Bearer，`/gateway/` 不继承 Web session 授权；令牌设置走常规鉴权/CSRF、no-store、只显示一次、不写浏览器持久存储。
- 外部客户端连接使用独立的 `/api/model-gateway/clients` 收据列表及 plan/commit/disconnect：用户指定 Host 绝对路径、明确模型，预览拥有的字段后再写入。断开由 Host 收据核验拥有字段，不从 UI 猜测删除。Android 通过现有单次浏览器授权入口管理已配对 Host，不把 Host 配置投影成 Android 本机账户。
- Web 房间回复复用 canonical timeline reducer 和既有消息/工具组件；只读取准确 context + executionID 的回复，授权与复杂运行控制链接到该独立会话。没有将 Provider delta 直接加入 transcript。
- 恢复意图由 Core `PrepareExecution` 在 savepoint 中先持久化 typed envelope 和固定 execution ID；Host envelope 保留原 canonical request/digest、模型版本与工具目录快照；native envelope 只含平台、配置/模型版本、parent 和原文本。`ClaimExecution` 不再生成新 ID，缺失意图拒绝。恢复查询原 source receipt，原请求幂等准入；不重新捕获配置、刷新失效工具目录或替换模型。`ReconcileExecution` 以真实 Run seal/source rejection，或本机真实配置/模型/parent 已变化的事实投影失败，不以超时或流结束猜结论。
- Web 集中检查 0 errors/0 warnings，411 项测试通过；Android JVM 编译成功，501 项测试中 499 通过、2 跳过。真实模型验收只读用户明确授权的 LAN 配置，并将所需凭据放入隔离 fixture；验收结束删除 fixture 凭据，不修改原用户配置/数据库。

## A1–A7 验收清单（2026-10-19，本轮证据）

| 总账项 | 已验证 | 仍缺/不扩大结论 |
|---|---|---|
| A1 水墨语义主题 | Web 静态检查/主题契约、Android 对比度 JVM 测试；Android 模拟器组件浅深色、320/720dp、1×/2× 字体截图已检查 | Web 无可用浏览器；Android 完整 Room 集成、读屏未验证 |
| A2 IM 多 Agent / owner 隔离 | 真实 LAN 的三独立 context、私有标记互不串线、两个非终态 Run 并存及冷启动回忆；草稿 owner 单测 | Web/Android 实际切换会话时焦点、滚动和忙碌状态未做设备交互验收 |
| A3 无收件人主控 | 真实无 @ 提交，auxiliary 成功选择职责匹配 Agent，Host intent→固定 execution ID→canonical completed seal；冷 recover 不新增 Run | 手机本机 owner 经配对 Host 的断网窗口与恢复仍待设备验证 |
| A4 显式收件人 | CLI `--to` 真实执行只进入选中 context；`/room <id> [@recipient] <message>` 真实 PTY 验收通过，名称前缀+Tab 转结构化 ID，未知提及拒绝保留输入；无 @ 仍经真实主控选择；解析回归 2/2；Web/Android 提及解析单测 | Web/Android 候选点击与键盘操作未实测 |
| A5 即时处理 / 授权 | 发送前 pending 草稿、接受/分派/执行/真实 seal 状态契约；Android 授权摘要独立于折叠工具；Web 工具需要注意时强制展开并保留独立会话入口 | 未做真实 Room 等待授权、批准/拒绝、切换期间提示可见性的全端端到端验收；不把执行中当授权状态证明 |
| A6 工具默认折叠 / 有序过程 | Web 组件/时间线单测与 Android 编译；等待授权/错误走外显注意状态 | 真实工具流持续更新时的滚动、展开保持和焦点未完成跨端视觉验收 |
| A7 组件/稳定输入/footer | CLI 真实 VT100 committed-frame 验收通过，132 帧/40 行保留、PageUp/down、Escape 正式取消 206ms、后续输入完成，idle composer/footer 各一；Web canonical 回复按 pinned snapshot 分页定位旧执行 | 固定尺寸 PTY 不证明 resize/中文输入法；减少动态效果、读屏、窄屏滚动帧耗时尚未测量 |

### 真实房间证据

- Web 与 Android 迟到回执均按 owner、不可变 input 和单调 revision 合并，旧 poll 不删除新消息、不将 completed 回退为 accepted。Android 专项编译/测试 `RoomComposerStateTest` 3/3 通过（含两项新回归），恢复调度同样消费合并后状态。

- `scripts/test_real_agent_room_isolation.py --concurrent` 完整通过，证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-isolation-do5nofrp`；包括 `unaddressed-dispatch/evidence.json` 与并发证据。
- 追加显式收件人后重跑通过，证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-isolation-8ikv1mav/explicit-recipient/evidence.json`。该次未传 `--concurrent`，不重复声称新一轮并发覆盖。
- 验收修复两处实际缺陷：会话 `automatic` 偏好映射到当前默认 reasoning tier；摘要校验和 source 注册使用不含 `request_digest` 的 semantic request。新增语义摘要单测通过。

### CLI 真实 PTY 证据

- `scripts/test_real_cli_terminal.py target/debug/nola target/debug/examples/terminal_acceptance_oracle` 通过。显式 test execution policy 关闭 tool/image/reasoning/sampling，实际使用获授权 LAN 文本模型；不是能力探测或修改用户模型声明。
- 证据 `/private/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-terminal-real-mb3h92cp`：132 committed frames，40 行均可见，独立 VT100 回放确认翻页；Run 依次 answered/cancelled/answered，取消 206ms、之后续行 0，后续输入成功。
- 所有包含输入提示的帧中 composer 和 Context footer 各一个；fixture 凭据清理成功，原配置摘要未变化，保留证据扫描未发现秘密。仅固定 100×24 PTY，不声称 resize/IME/读屏通过。
- `python3 scripts/test_real_agent_room_isolation.py --binary target/debug/nola --service target/debug/nola-service --config /Users/gaojc/.nola/config.json --interactive-oracle target/debug/examples/terminal_acceptance_oracle` 通过（隔离 schema 82 fixture）。证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-isolation-speb89c4/room-pty/evidence.json`：未知 @ 显式拒绝且输入保留，`@b` 候选经 Tab 选择 B 并完成，无 @ 经真实主控选择 A 并完成，idle 输入/footer 各一个。普通聊天 owner 未切换；真实权限审批交互尚未验证。
- 实测发现 Room 命令错误会退出交互 shell，现已在交互命令边界显示错误并保留 shell；验收 fixture 显式禁用 Skills，避免把未物化的内置 Skill 当作模型/房间协议故障。未更改用户 Skills 或配置。

### 网关 HTTP 证据（不是视觉通过）

- `scripts/test_real_model_gateway.py` 通过；隔离 fixture 导入账户/连接、独立 token、模型目录、真实 LAN SSE 转发、持久 usage 重开、token 撤销均验证。
- 透明观察中继只转发 `192.168.31.18:8081` 真实响应，无模拟生成；上游与网关 7,422 字节逐字节相等，仅 1 次物理请求，未创建 Agent Run。
- 证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-real-gateway-gqf5dulr/evidence.json`：`completed`、输入 65/输出 31/缓存 60，未知字段为 null；重启前后记录相等。未写真实外部客户端配置。

### Room 授权等待专项审查

- 按 apple-design `feedback.md › Best practices` 的就近状态反馈原则检查，Web/Android 遵循自身平台控件，不套用 Apple 布局；工具过程默认折叠不能隐藏待授权事项。
- CLI 实际缺口：交互 `/room` 同步消费 SSE 到结束，权限 challenge 被丢弃，导致等待时没有确认入口。已新增仅交互调用注入的授权回调：读取真实 challenge，严格核对 Host/client/context/canonical Run，显示安全审查文本并复用终端权限面板；一次/会话/拒绝/说明分别提交原 nonce 的 typed resolution，Escape 拒绝，不隐式批准，不切换普通会话 owner。集中构建后 `interactive_room_mentions_tests` 3/3、`room_permission_owner_tests` 1/1 通过；尚未宣称真实权限 PTY 通过。
- Android 无同类缺口：房间成员状态行在工具折叠之外显示等待权限，点成员检查进入原 context 的独立 `RoomPermissionCards`，可允许一次/会话、拒绝、提供说明；Stop 使用原 context owner 和 active response ID。`ChatRunRegistryTest` 9、`ChatViewModelPermissionTest` 18、`AgentRoomPresentationTest` 8，共 35/35 JVM 测试通过（2026-10-19，7s）。覆盖迟到权限回调仍绑定原 owner、删除只退休该 owner、许可 payload 与房间投影；不等于物理设备触控/视觉验收。
- Android 系统权限/文档/照片/HealthConnect 启动器由外层 `ChatScreen` 在 `showAgentRooms` 分支之前注册；进入 Room 后仍保持组合，结果按原 request ID 路由，不需要复制另一套 launcher。
- 复用 Host 权限通道补验：`RUST_MIN_STACK=33554432 target/debug/deps/nola_cli-bc729b9b5cf0d1fa web::host_permission_broker_tests:: --test-threads=1`，9/9 通过，覆盖真实 waiter 的允许/拒绝只恢复原调用一次、说明不授权、过期 nonce、断流、多 ask 反序和安全预览。Core `real_host_permission_interruption_repeated_resume_preserves_original_call` exact 另 1/1 通过（6.58s）。首次未设置栈大小的宽 filter 在该深栈 Core 用例 stack overflow，按 32MiB 重跑通过；未隐去失败，也未为测试改生产逻辑。
- Web 原 `RoomReply` 只有正文与完整 context 链接，没有独立授权投影；由主代理接现有 RunObserver/ChatPermissionPanel，避免重复实现。
- 真实权限 PTY 准备发现 `write_file` 无安全预览而无法确认；按主线补齐仅原请求准确路径、覆盖/预览模式、UTF-8 字节数和 SHA-256，不暴露写入正文。拒绝未知参数、错误类型、被裁剪路径和非法旧版本摘要；新增原内容改变必须改变摘要、正文不泄露等回归。`scripts/test_real_room_permission_terminal.py` 使用隔离 workspace 的 existing.txt，数字 1 不带 Enter；审批前字节不变、审批后原 executionID 单 Run seal、footer 单份均为独立断言，等待最新 CLI binary 实跑。
- 最新 CLI binary 构建通过；安全预览新增测试 1/1 通过，日志 `/tmp/nola-write-permission-preview-test.log`。真实 Room 权限 PTY 第一轮在 seed 模型请求即失败，未进入权限环节：证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-room-permission-pty-ubow0262`。无凭据只读 `curl --connect-timeout 5 --max-time 10 http://192.168.31.18:8081/v1/models` 连接超时（exit 28、HTTP 000、5.010833s）。临时文件未写、fixture 凭据已清理、原配置摘要不变。权限面板/数字确认/原 Run 继续的真实 PTY 断言仍未覆盖，不记通过；服务恢复后重跑原脚本。

### Android 隔离组件视觉验收

- 完整页面补验采用既有 `ActivityScenario(MainActivity)` 与真实 `AndroidApplicationRuntime/AgentRoomRepository`：新隔离安装创建两个空配置Agent/房间，只测`@`与草稿切换，不发送、不配置模型。首轮实际发现默认Runtime启动失败：系统`no_backup`目录为0771，而提交日志拒绝组可写目录（`instrumentation-r1.log`位于`/tmp/nola-android-room-product.YU1AW6`）。不能以此前手工0700的隔离fixture证明默认路径正常。
- 最小安全修复保留`no_backup/nola-product.db`路径和全部内容：仅默认目录核验真实父目录位于app数据目录、lstat非symlink且当前UID所有，再以O_RDONLY|O_NOFOLLOW打开；fstat核对目录类型及UID/dev/inode不变后fchmod0700。显式外部数据库路径不自动改权限；不迁移、删除或重建数据。新增真实平台权限收紧/内容保留、symlink及错误父目录拒绝回归。
- 完整页面最终使用本轮构建的arm64 Core/JNI，2/2通过（8.204s）：`/tmp/nola-android-room-current-r4.J7ICRR/instrumentation.log`。真实MainActivity导航、@选择和跨Room草稿/收件人恢复，截图同目录room-full-*.png，已目检。此前旧JNI失败及测试query/placeholder断言失败保留，不伪称产品回归；未发送模型消息。

- 后续复验：`emulator -list-avds` 有既存 `nola_avd`；原 `emulator-5580` 不在线。使用全新 `/tmp/nola-android-room-followup.LhsHUc` userdata/cache 启动 read-only/no-snapshot 模拟器，`ro.kernel.qemu=1`、`sys.boot_completed=1`；未查询实体设备，未下载镜像。当前 UI APK 构建成功（28s，跳过 Rust，仅复用 native 库），同三个 Compose instrumentation 8/8（17.545s），日志该目录 `build.log` / `instrumentation.log`；Room repository/presentation/composer JVM 16/16，`jvm.log`。结束已通过显式 `adb -s emulator-5580 emu kill` 关闭本轮模拟器。
- 新截图目录 `/tmp/nola-android-room-followup.LhsHUc/screenshots`：浅窄 `room-top-false-320-1.0.png`、深宽 `room-top-true-720-1.0.png`、浅宽2× `room-top-false-720-2.0.png`、深窄2×授权 `room-permission-true-320-2.0.png`。四张已实际目检，未发现横向裁切；2×窄屏按钮需滚动，测试确认三种决策各自可点击。依 apple-design 的反馈、布局和可读性原则复核，保留 Material 控件规范。
- 范围仍是产品 `RoomMessageRow` / `PermissionDialog` 的合成状态组件，不是完整 `AgentRoomsScreen`。完整页直接依赖拥有约20个运行服务的 ChatViewModel，现有fixture没有整页注入入口；本轮未为截图新增第二状态/执行架构。`@`、owner切换、迟到回执由上述16项JVM覆盖，不宣称模拟器完整房间输入/@/授权Run恢复通过；系统TalkBack未验收。

- 用户通知本地模型停用后已暂停 LAN 探测与真实权限 PTY，不切换其他模型。复用已有 `nola_avd` Android 35 镜像，以 `emulator-5580`、read-only/no-snapshot 和独立临时 userdata/cache 启动；不查询实体设备、不安装新镜像、不装入用户配置或凭据。
- `RoomPresentationVisualTest` 使用真实 `RoomMessageRow` / `PermissionDialog` 与合成展示数据，覆盖浅/深色、320/720dp、1×/2× 字体；允许一次、会话允许、拒绝分别触发自己的回调，允许一次触控区域至少 48dp。没有实际模型调用、文件授权或设备权限申请。
- 构建：`./gradlew :app:assembleDebug :app:assembleDebugAndroidTest -x buildRustCore -Pandroid.injected.build.abi=arm64-v8a --no-daemon --max-workers=1 -Dorg.gradle.jvmargs='-Xmx2g -XX:MaxMetaspaceSize=512m'` 成功。仅 UI 构建，复用已有 native 库，不作为当前 Core 集成验证。安装的是 `app/build/intermediates/apk/debug/app-debug.apk` 与 `app/build/intermediates/apk/androidTest/debug/app-debug-androidTest.apk`，不能使用本次未更新的 outputs APK。
- 指定上述模拟器执行 `RoomPresentationVisualTest,PermissionInstructionUiTest,PendingPermissionNavigationUiTest` 共 8/8 通过（10.613s）。初次截图发现测试仅改 Compose density、AndroidView 仍用设备 density；已将 fixture 的 Context density/fontScale 一并对齐，最终 Room 四种尺寸/字体用例再次 4/4 通过（6.965s）。这是测试装置修正，不冒充生产字体缺陷。
- 最终截图：`/tmp/nola-android-room-ui.UMGzRw/final-screenshots`，旧 `screenshots` 目录已被最终结果取代。代表文件：`room-top-false-320-1.0.png`、`room-top-true-720-1.0.png`、`room-top-false-720-2.0.png`、`room-permission-true-320-2.0.png`。目检无文字横向截断；2× 窄屏允许/拒绝可纵向滚动到达，不宣称全部按钮同时位于首屏。
- 角色色计算：浅/深正文对表面分别 14.62:1 / 13.70:1，主按钮文字分别 6.22:1 / 8.56:1；不外推为全部组件对比度审计。完整 Room 路由、owner 切换、真实待授权 Run 继续、IME、TalkBack、减少动态效果仍未由本轮组件测试验证。

### 模型管理平台边界（实现前设计）

- CLI 本机连接使用现有 Core/Host：connections 读脱敏目录，import 逐项预览确认且默认不启用，probe 单次费用确认，override 显式用户声明。本轮补 discover（仅 GET 目录）与 configure（完整启用集合、选中目录键、revision CAS），闭合导入后无 CLI 启用入口的断点。configure 指定选中键会替换同 provider/runtime 的选中 profile，帮助明确告知这一副作用；`none` 不选择其他连接模型。不自动付费验证。
- CLI 来源认证不另造登录协议：新增 manage 输出当前端口的 loopback 管理地址与启动 Host 的准确命令；用户在本机浏览器进入已有来源管理页。命令不自动启动 OAuth、不打开远端地址、不宣称登录后可推理。
- Android 连接清单属于本机；Host 文件导入/目录/探测/网关仍经既有配对授权浏览器 handoff，明确命名操作范围。来源登录接口是 Host loopback-only，手机页面提供“在 Host 本机操作”的说明，不能转发成手机 OAuth 或暗示配对授予来源认证。未配对时说明为何管理按钮不可用。
- 页面延用 Material TextButton、现有语义色和可换行 body 文案；紧凑/宽屏都保持同一能力。`managing-accounts.md › Best practices` 的清楚说明账户收益与 `privacy.md › Protecting data` 的不自创认证原则用于边界；不将 Apple 导航惯例搬到 Android。OpenCode/Codex/Kimi 来源账户管理与 API key 连接导入分开，来源登录成功不等于 Nola 推理支持。

#### 落地与验证范围

| 能力 | CLI | Android |
|---|---|---|
| 连接/账户清单 | `model connections`，账户凭据引用不输出 | 本机 Core 清单；不冒充已配对 Host 清单 |
| 配置导入 | `model import`，显式绝对路径、逐项确认；凭据/环境读取另行显式授权 | 既有配对加密一次性 handoff 到 Host `/models?tab=api`；不读取手机或 Host 的隐式凭据 |
| 目录与启用 | 新 `model discover <connection> <revision>`、`model configure <connection> <revision> <selected-key或none> <完整enabled键JSON数组>` | 同上 Host 管理页；不新建手机端目录适配器 |
| 能力探测/声明 | 已有 `model probe` 明确费用确认，最多一次；`model override` 显式声明，无请求 | Host 管理页验证；按钮及说明明确可能费用，非自动触发 |
| 来源认证管理 | 新 `model manage` 输出本机 Host 管理地址及启动命令；无直接 CLI OAuth 子命令 | 常显 Host 本机浏览器操作说明；不能经手机远端调用 loopback 来源登录接口 |

- Rust 新解析回归覆盖正 revision、准确含冒号目录键、完整空集停用、重复/空键/错误类型/选中键不属于启用集拒绝；交账户组统一编译验证。Host 安全重定向测试补 `/models?tab=api` 保留断言，既有跨源/API跳转拒绝不变。
- Android `:app:testDebugUnitTest -x buildRustCore --tests 'com.nola.data.remote.api.WebHostApiServiceImplTest' --tests 'com.nola.ui.screen.models.WebHostConnectionWorkflowTest' --no-daemon --max-workers=1`（JVM 2GiB）成功 26s，13/13 通过；含模型管理 query 保留/同源地址、认证绑定/连接状态边界。没有执行模型请求、真实 OAuth 或物理设备操作。此轮不将 JVM 地址测试描述为真实浏览器来源登录通过。

## 2026-10-19 TokenUsage 可选计数接线

- Core TokenUsage 的六项计数是 nullable，不再带 cache `*_reported`；与仍采用numeric+reported的 ContextUsageSnapshot 分属不同协议，不作兼容双写。Android 原model/compaction completion两条接线仍发送旧字段且把未报告计数变0，已移除；新增统一`toCoreUsageJson`保留null/真实0，request_count=1，已知lower-bound仅用于预算。
- Android provider→LlmStreamUsage→AgentCoreTokenUsage及非流式ModelOutput均改nullable。Responses/Chat缺失总量不以局部input+output猜完整total，reasoning仅使用provider明确字段；placeholder不伪造0用量。Core仍负责累计，客户端不新增累加器。
- Web现代会话UI消费独立ContextUsage/PromptCacheUsage契约，cache未知显示未知、保留reported/unreported覆盖；gateway字段用`?? '—'`保留真实0。CLI footer读取ContextUsageSnapshot且保留estimated/缓存reported，未发现直接TokenUsage归零展示，不改其当前协议。
- Web定向`node --test src/lib/conversationCacheUsage.test.ts src/lib/runContextUsage.test.ts src/lib/conversationCacheReader.test.ts` 15/15通过，新增真实0与部分累计覆盖回归。Android最终`testDebugUnitTest -x buildRustCore --tests com.nola.core.TokenUsageUnknownTest --tests com.nola.llm.provider.OpenAiResponsesAdapterTest` 7/7通过（13s，`/tmp/nola-android-usage-null-tests-r2.log`），含Chat/Responses部分和null provider用量。均为隔离协议fixture，未运行native Core集成、真实模型/OAuth或物理设备。
