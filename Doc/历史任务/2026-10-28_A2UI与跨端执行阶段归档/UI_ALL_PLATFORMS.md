> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

> **单版本开发约束：仅保留当前架构、接口、协议和数据格式；不新增兼容层、迁移或转换，不自动删除、重建或改写用户数据。**

# 全平台 UI 优化执行清单

2026-10-19 用户追加：全面优化所有平台 UI。本清单扩展现有任务，不以已完成的 Room 组件测试替代整个平台验收。

## 当前验收门槛（2026-10-25，尚未收尾）

本节是当前状态；后文“本轮状态”和“最新实际页面检查”保留早先批次的历史记录，不能据此宣布本次全平台完成。

| 范围 | 当前直接证据 | 尚缺的证据 |
| --- | --- | --- |
| Web Goal 控制 | `/tmp/nola-real-goal-controls-fixed/report.json`：真实 Chrome 开始、暂停、刷新恢复、继续、取消全部 HTTP 200；已修会话模型解析遗漏 | 长目标实际自动执行、故障后继续，不以状态按钮通过代替 |
| Web 管理导航 | `/tmp/nola-web-parallel-sections/report.json`：37 次真实点击 | 所有有数据/错误/窄屏组合；真实发送另有180秒超时失败，未隐去 |
| iOS 输入与配对 | 新隔离模拟器 `/tmp/nola-ios-fresh-focus.xcresult`：真实键盘 URL/token 来回焦点通过；旧模拟器同代码失败 | Host 密钥触发系统保存密码遮挡已修，完整远程多任务复测仍进行 |
| Android | 隔离 user 11 的 workspace 实际点击、分区切换、斜线提示与帮助通过 | 新版本真实模型、停止再发、恢复与窄屏截图修复后复测 |
| Mac A2UI | 实际输入提交正确 ACK；重复同内容 publication 返回 changed=false，revision保持3 | action Run最终 incomplete；旧测试仅等 idle会假绿，已要求成功终态断言，不算完整通过 |
| 远程子任务 | 既有同一会话冷恢复与两个子任务审批/汇总通过，详见 UI_CLI_AUDIT.md | 当前全部平台新会话、长任务、故障、取消和恢复完整闭环 |

任何“通过”仅限该行所列操作；工具请求成功、Surface更新、Run结束和业务完成必须分别核验。

## 共享协议与执行边界收敛（2026-10-25）

用户要求：能复用的尽量复用，协议和状态规则收进 Core，不能继续各端修补。

### 当前实施约束与缺口

共享不是把各端函数移到公共目录：同一输入必须由同一 Core 协议给出归属、允许操作和状态变化，各端不得重新计算完成、恢复或授权结论。

| 规则 | 唯一实现位置与当前状态 | 宿主/客户端保留职责 |
| --- | --- | --- |
| 冻结提交与恢复资格 | Core `ObserveSubmissionFrontier` / `ValidateRunRecoveryAuthority`；Apple、Android 已接入 | 身份认证、传输、保留精确提交凭据 |
| 冷取消与进程身份 | Core `InspectCancellationProcessOwners` / `ValidateCancellationProcessObservations`；CLI、Apple、Android 接入 | 读取系统 boot/start/PID 事实；无法观察时明确 Unknown，不推断死亡 |
| Goal 回合与控制 | `nola-runtime-driver::goal` 统一协调，Core 校验精确 Goal/contract/Run/revision | 唤醒、实际模型/工具执行；HTTP 202 只表示排队 |
| 用户问题与回答 | 已有 Core `UserInputWait` / `RespondToConversationWait`；Web 呈现入口与回答后 Goal 恢复仍缺 | 选项/文本输入，携带 wait ID、revision、schema digest、稳定 command ID |
| 思考流 | Core 统一决定请求能力与展示策略；发现 Android 收到 reasoning 但被计划禁止转发，尚未修复 | 解析实际 provider 流并如实交给约定入口，不擅自覆盖 Core policy |
| 视图瞬时状态 | 留在平台 | IME、焦点、草稿编辑、滚动、动画、系统组件 |

回答等待的必要不变量：回答只能恢复其精确归属的 Run；Goal 保留同一 contract 和 execution_run_id。Core 必须在回答生效后决定是否从 NeedsUser 继续，Host 仅唤醒既有 worker。不得由浏览器猜测后另发 Start/Resume，不得将普通回答当作工具授权。取消已提交、过期 revision、重复回答和子任务回答均需覆盖。

当前真实阻塞：无字节 SSE 的 Goal 暂停已进入 Cancelling，但尚未及时取得实际停止确认；不能以按钮状态变化算通过。移动端新协议冷取消、iOS 原会话压缩仍在真实操作复测。测试分别记录 Core 合同测试、HTTP/provider 测试与实际 UI 操作，禁止互相替代。

后续实测更新：共享 pump 已接入已提交 Cancel 的精确执行中断，Goal 隔离回归 21 项（包含真实 HTTP 静默 SSE 与权限等待）通过，浏览器仍需等待实际 Run 启动再测控制，快速点击不算执行验收。iOS 原会话压缩与压缩后原事实回忆成功，流式首文本后停止的 UI 测试和 cancelled seal 均通过。Android 冷取消 seal 成功，但随后主线程 Core 重放引发 ANR，端到端体验仍未通过。

冷恢复发现的共享缺口：Interrupted/Resume 的旧请求必须先由 Core 生成新 request 身份，才能进入 successor claim。新增 `PrepareRunEffectRecovery` 在同一事务验证冻结 Run、选择已授权的 interrupted 尾 attempt，并调用现有 Core ResumeHostEffect；不会放宽调度 SQL 重放原请求。已编译通过，正在真实失败数据库的隔离副本上验证；移动端接入和完整复测尚未完成。

- **Core Domain / Application**：远程提交 frontier 的类型、身份验证、已有 source receipt 归属、原子配置 revision/可见 parent 读取；冻结提交仍按原 Core admission 校验，冲突不得自动 rebase。已落地 `SubmissionFrontierRequest` / `SubmissionFrontier` 与 `ObserveSubmissionFrontier`，复用既有 snapshot repository。
- **Core 执行协调器（进行中）**：Goal 的 lease、回合、恢复、暂停/取消、终态判据只保留一份。已从终端私有代码抽出共享入口；进一步迁入 Core runtime 层，Host 通过 typed ports 提供模型执行和输出。仅放在 CLI 模块不算跨平台完成。
- **Host**：HTTP认证/加密、实际provider/工具调用、系统权限执行、进程/平台唤醒；不能自行判定Goal完成，不能只写running状态就算开始执行。
- **平台客户端**：呈现Core状态、系统键盘/焦点、操作输入；远程新提交只冻结Host frontier，已有bound submission/recovery不得重读后覆盖。不得用本地镜像配置revision代表Host。

确认的未完成项：Web Start/Resume只提交Goal projection、缺持续执行；Mac native入口曾绕过终端Goal循环；需接入同一Core协调器及持久唤醒/恢复后，再重跑真实任务验收。

## 设计结论

沿用 [水墨青山设计系统](../../UI和交互设计/DESIGN_SYSTEM.md) 的颜色、字号、间距与组件配方，不重新发明主题。产品服务长时间阅读与多 Agent 协作：答案与成员身份优先，执行过程退后，待授权事项明确可见。统一的是语义和层级，不是把 Android、终端强行画成 SwiftUI。

依据 apple-design：layout.md 的 Visual hierarchy 强调对齐与渐进披露；typography.md 的 Supporting Dynamic Type 要求字号增加时布局重排；motion.md 的 Providing feedback 要求常见交互避免多余动画；feedback.md 的 Best practices 要求状态就近展示。上述原则用于审查，具体水墨配色和组件尺度是 Nola 的设计选择。

## 页面覆盖及责任

| 表面 | Apple：macOS / iOS | Android | Web | CLI |
| --- | --- | --- | --- | --- |
| 导航与会话列表 | Apple agent | Android agent | Web agent | 主代理：会话选择 |
| Chat / Room / @ / 草稿 | 同上 | 同上 | 同上 | 主代理：输入与历史 |
| 工具过程、错误与授权 | 同上 | 同上 | 同上 | 主代理：提示与决策 |
| 模型账户、导入与设置 | 同上 | 同上 | 同上 | 主代理：选择与反馈 |
| 文件、引用、预览 | 同上 | 同上 | 同上 | 主代理：文本链接与结果 |
| 空态、加载、失败、恢复 | 同上 | 同上 | 同上 | 同上 |

各平台先登记具体文件和可验证缺陷，修共享组件后检查所有消费者，不只单页换色。Apple/Android/Web 分别写 UI_APPLE_AUDIT.md、UI_ANDROID_AUDIT.md、UI_WEB_AUDIT.md；主代理汇总，不覆盖既有修改。

## 优先级与验收边界

1. 可用性：关键操作可达，键盘/焦点不丢失，字号放大不截断，授权不被折叠。
2. 结构：导航、阅读列与输入区对齐；去除多层无语义卡片和重复状态；模型设置主次清楚。
3. 视觉：语义色配对、正文层级、空白、边界、选中与禁用状态统一。
4. 动效：只更新变化的局部，减少动态效果生效，不延迟文本显示、不播放历史进入动画。
5. 代码整体修改后集中测试并检查实际截图；区分源码审查、组件测试、整页操作、真实模型验收四种证据。

最低检查矩阵：浅/深色，手机窄屏和大字号，桌面窄/宽窗口，Web 200% 缩放，长中文/英文/文件名，空态/运行/授权/失败，手动滚动与切换会话。CLI 另测窄终端、resize、无颜色、中文输入和单一 footer，不使用大面积强制背景色。

当前限制：本地模型停用；原 Web 用户库格式被当前 Core 拒绝，暂只使用独立预览库，不能将空预览当作历史恢复完成。实体设备不轮询，测试隔离，macOS 使用 Test 配置。未测量帧耗时或读屏时不得标记通过。

## 本轮状态

- Apple、Android、Web 本轮共享组件与主要页面优化已实施，分端验证结果如下；不是所有窗口和系统无障碍的完整验收。
- CLI 已修复固定前景/背景色造成的终端主题不适配；终端模块71/71回归通过，保留输入区所有权与取消协议。最新真实隔离PTY已验证40列会话选择、三组resize、单一输入区和中文草稿，确认Run数为0；不是Terminal.app人工像素验收。
- 本轮实现与集中回归已收口；真实模型、原用户库恢复及完整系统无障碍验收仍未完成。

### 最新实际页面检查

- Android 完整 Room/@/草稿所有权2/2，UI组件12/12、JVM16/16；主代理已查看实际Room与最终模型页截图。Room早期组件截图的底部空白经整屏截图确认是未被组件捕获的系统键盘，隐藏IME后输入区回到底部；不是据猜测修改布局。
- Chrome 独立临时profile生成1440×1000 Chat截图，主代理发现空会话仍有禁用大输入区与空详情面板，已交Web修复并等待新图。首轮Chrome写出截图后退出2（teardown watchdog），不能记整个命令通过。390×844模型页截图命令退出0，但只捕获加载态，不能据此声明模型管理全页验收。
- Web在首轮429项检查后继续修截图发现的空态问题，最终检查计数以修复后的结果为准。
- Web空态修复后430/430通过。Chrome使用独立profile以1440×1000和390×844重新截图，两个命令均退出0，主代理已查看 `chat-wide-fixed.png` 和 `chat-narrow-fixed.png`（`/tmp/nola-web-visual.0Y8Ubk/`）：空详情与无效输入框已消失，新建按钮可见。这里只证明空态布局，不证明有内容会话或点击提交。
- Apple最终Mac49/49、iOS4/4通过；详细窗口与截图限制见UI_APPLE_AUDIT.md。Android最终证据见UI_ANDROID_AUDIT.md，不将组件截图扩大为全部系统无障碍通过。
- 主代理另行查看iOS320pt/AX5深色管理组件图（`8AF21815-B4CA-4440-9631-125370C97384.png`）：标题与说明占满可用宽度纵向换行，不再被左右图标挤成两字列。该图仍是组件组合，不是设置导航操作证据。
- Web独立Chrome真实交互已验证两会话草稿隔离、Tab导航及320/1440视口；不调用模型。后续发现CSS `zoom:2`不改变浏览器布局视口，不能代替原生200%缩放，且仅断言根无横溢会漏掉窄列可读性问题：该模拟不计缩放验收通过，补列宽检查。截图重复区经DOM唯一性核对不是重复挂载；浏览器绘制证据限制仍保留，不凭截图异常添加布局hack。
- Web会话列表安全错误出口的Rust回归1/1通过（`cargo test -p nola_cli --lib public_web_error_tests --quiet`），确认不会将服务stderr或私有路径直接回传；这不解决原用户库格式不匹配。
- 最终Chrome交互报告 `nola-web-ui-acceptance.lE89Z2/result.json` 已由主代理读取，8项成功、blocked/errors/visualIssues为空；主代理再次目检最终320px深色设置图，按钮正常、无重复画面。有效范围为两会话未发送草稿与键盘导航、三页宽窄布局；不是所有设置子页/真实模型回复/原生200%缩放通过。复用验收脚本正在补充端点实际数据库归属的fail-closed核验，避免只凭一个临时库路径存在就允许创建测试会话。
- 复用脚本安全核验已完成：运行前与允许的POST前核对唯一监听进程、可执行文件、启动身份及临时数据库绑定，失配拒绝运行且不输出环境秘密。主代理复核实现并重跑guard测试3/3通过；没有因此再次创建会话或调用模型。

#### 移动 embedded 多 Agent 能力范围更正

iOS/Android 既往真实父子任务、指导和子结果验收均为 paired CLI Host 执行；embedded 本地 `spawn_agent` 尚未接入共享子 Run runner。Core 有 admission/lease/terminal 端口，CLI 有实际 process executor，mobile-bindings 尚无等价 in-process 适配。不能以隐藏控件或远程成功代替本地闭环。两端 `cancel_task` 合成成功已改为 exact Core 持久请求，Transfer 无可信执行通道则明确失败；本轮新修改待统一构建验证。
