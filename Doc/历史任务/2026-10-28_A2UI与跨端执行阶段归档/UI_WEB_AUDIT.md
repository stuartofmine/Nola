> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

# Web 水墨体系全页审计

2026-10-19；范围：Svelte shell、共享控件、Chat、Rooms、ModelCenter/API账户、Settings、DataSources、Capabilities、Automations、Results、Dashboard、Login与资源预览。保留所有API、账户owner、权限和执行事实。当前是源码可验证审计，不把构建当窗口视觉验收。

## 方向与依据

纸白阅读面、墨色正文、山青交互；正文与任务身份优先，管理信息渐进披露。沿用 DESIGN_SYSTEM.md 六基础角色，不再引入第二色板。桌面系统字体14/20表单、16/25聊天，窄屏表单16、聊天17；导航与控件保留Web语义、浏览器缩放和键盘操作，不仿制macOS菜单栏或iOS tab bar。

apple-design 已读取 accessibility（Vision、Mobility：支持200%文字、键盘与足够目标）、layout（Adaptability：布局适应文字与可用空间）、typography（Conveying hierarchy）、color（Inclusive color）、designing-for-macos、cross-platform、sidebars、entering-data、motion（“Make motion optional”）、dark-mode。具体水墨色值和尺寸属于Nola设计决策。

## 可验证问题与实施

| 级别 | 源码事实 | 修复方向 |
|---|---|---|
| High | Settings选中标签用未定义accent-text fallback白字，深色accent为浅青 | 使用on-accent，清除未定义text变量 |
| High | theme root锁16px；全局按钮disabled整体opacity .5，textarea缺共享边界；tab按钮最小130px会挤窄窗 | 百分比root、语义disabled、统一field、tab可换行与零最小宽 |
| High | 共享按钮缺hover/pressed反馈、outline使用装饰divider；滚动容器/长文本没有统一约束 | 明确控件outline、focus可见、文字断行与局部滚动 |
| High | Chat桌面用户消息铺满列、正文18px且读取宽度780；composer装饰边界/大固定起始高度 | 用户85%靠尾、16/25正文720阅读宽、880列、明确surface composer |
| Medium | Markdown行高1.7、代码padding14/表格9不符当前配方；PDF白底350固定高度 | 统一阅读节奏、局部滚动、预览适应视口并保留独立打开 |
| Medium | 页面card14/control7与radius10并存；motion最慢380 | control8/container12/message16，100/180/220低干扰，无整页动效 |

布局：宽屏 `导航 | 主栏（管理≤960、聊天≤880） | 详情按需`；窄屏 `导航按钮 / 当前页标题 / 单列正文 / 当前输入`。签名只在山青选择和稳定身份，去掉额外装饰层，不增渐变/玻璃。

## 验证门禁

修改后运行Svelte check、Node tests、Vite build；补源码契约与色对计算回归。需后续真实窗口覆盖320/760/1440、浅深色、200%缩放、长中英文、键盘焦点、空/加载/错误/有内容、预览。当前Chrome5001仅HTTP预览可用，未取得受控浏览器视觉证据，不能宣称视觉通过。

## 本轮实施与页面覆盖

- theme：百分比root、14px默认表单、共享textarea/checkbox、32/44控件、明确outline、可读disabled、hover/pressed/focus、可换行tabs、960管理列、12容器/8控件、100/180/220ms、reduced-motion/forced-colors。不增加全页进入动画。
- shell/Sidebar：七语言跳至主内容链接，Chat/Login/普通页都有真实目标；长导航名可换行、当前项轻底色、Settings入口hover/选择明确。
- Chat/Markdown：16/25桌面、17px窄屏阅读；880列、720正文、85%用户气泡；pending以虚线而非整块降低对比度；composer surface/outline，代码表格局部滚动，引用/间距归一。
- Rooms：窄屏房间导航为自适应网格且限制30dvh内部滚动，不再被全局横向nav规则挤压。消息/权限/恢复流程不变。
- Models/API来源与Settings：共享tabs/forms/card改善全部模型管理；清除OpenCode未定义border-color与Settings未定义accent-text/text。原有端点隔离、revision、成本确认、来源owner说明保留。
- DataSources/Dashboard：已具备窄屏统计堆叠、长URI换行和显式错误，保留；接受新共享卡片/控件/页面密度。
- Automations/Results/ModelTest/Capabilities：保留原有响应式筛选、容器断点、空/加载/失败与确认流程；共享字段/对话框约束防止窄窗横溢，Capabilities编辑框改真实控件outline。
- Login：surface阅读面与容器尺度，保留敏感字段不预填、焦点错误反馈及原登录契约。
- 资源与知识：PDF以视口高度限制且保留独立打开链接，附件禁用说明不透明度衰减；Knowledge输入去掉未定义text变量。没有改变PDF加载权限或sandbox。

## 验证结果

最终统一命令 `npm run check && npm test && npm run build` 退出0：Svelte 0 errors/0 warnings；Node 429/429；Vite成功。新增designSystem三项验证浅深色色对、共享可达性/布局契约及预览owner调用保留。`git diff --check`通过。首轮日志 `/tmp/nola-web-ui-check.log`、`/tmp/nola-web-ui-tests.log`、`/tmp/nola-web-ui-build.log`；最后统一进程4180也成功。

仍有非失败构建提示：公共api chunk约528kB；本轮不为消警告扩大模块重构。没有窗口截图/200%缩放/读屏/帧率证据，因此状态是“代码与自动检查通过，窗口视觉验收待执行”，不是全端UI完成。

## 首张真实Chrome截图后的修正

已目检 `/tmp/nola-web-visual.0Y8Ubk/chat-wide.png`（1440×1000、临时数据库、深色空态）。Chrome写出截图但退出2、teardown watchdog，因此只有真实渲染artifact证据，不能记录浏览器流程成功。该图暴露High问题：无会话时巨大disabled composer与空详情同时显示，重复“选择或创建会话”，正文没有主动作，聊天外壳与composer叠边框。

已修：无active时正文显示复用 `ChatConversationManager.createConversation` 的明确主按钮；不挂载composer、详情栏及其开关和空工具条；初始化失败保留错误与retry，不当成功空态。新建失败回执以原host/generation绑定，成功仍由原manager选择会话，不新增API或owner路径。聊天外壳去掉整面板边框，保留真实控件边界。加载态SSR回归改为断言composer/详情不出现，并保留全部实际详情组件独立渲染测试；新增源码契约回归。

修正后Svelte check 0/0，Vite成功（`/tmp/nola-web-empty-chat-check-r2.log`、`/tmp/nola-web-empty-chat-build-r2.log`）；Node 430/430、0失败，记录 `/tmp/nola-web-empty-chat-tests-r2.log`。修后真实截图仍待主代理捕获，不能以修改代码推断视觉结果。

## 独立真实Chrome交互验收

新增可复用脚本 `web/playwright_runtime/ui_preview_audit.cjs`，使用仓库现有playwright-core与系统Chrome，不安装依赖、不连接用户profile。仅接受明确指定的 `/tmp/nola-web-preview.*/state.db`，网络限制为127.0.0.1:5001；POST只放行创建会话及源码确认的只读observe/来源owners，禁止其他写入、模型、登录和WebSocket。测试未按Send。

最终命令：`NOLA_UI_PREVIEW_DB=/tmp/nola-web-preview.kF34Ss/state.db NOLA_UI_REUSE_FIXTURES=1 node web/playwright_runtime/ui_preview_audit.cjs`。实际进程15706退出0；`/tmp/nola-web-ui-real-acceptance-final.log`；证据目录 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-web-ui-acceptance.lE89Z2`（result.json＋6张图）。新执行默认创建2会话；本次r2已实际创建2个fixture，后续复用它们。r1因脚本URL参数误写多留1个临时会话，没有删除数据。后续修正等待真实URL切换，不用固定睡眠掩盖异步。

通过的8项：两份中英文未发送草稿往返切换隔离；真实键盘Tab到新建、会话与composer；Chat/Settings/Models在1440与320视口无根横向溢出。Chat实际列宽809/320，新增最小可读列宽判据防止“没有横溢但挤成单字”的假绿。所有页h1和main各1；blocked=[]、pageerror=[]。宽屏浅色与窄屏深色图已目检；实际Settings选中前景#9AC7AC/底#233C2F=6.33:1，普通tab#E5ECE6/#18211E=13.70:1，不是截图过渡时的白字灰底。

CSS zoom=2仅是模拟，不改变innerWidth/响应式断点，确实会压缩Chat三栏。早期仅检查根overflow的r5/r8不能算200%通过；脚本现默认不做该模拟，显式 `NOLA_UI_CSS_ZOOM=1` 时可读列不足会失败。CSS zoom后的截图还出现重复绘制，DOM却只有一个heading/main；最终不使用此模拟的新Chrome图无重复。**原生浏览器200%缩放仍未验收**，没有以CDP缩放冒充，也未为错误模拟添加生产布局hack。真实模型回复、读屏、所有Settings子页与所有控件状态仍未全面验证。

复用安全收口：新增 `ui_preview_owner.cjs`，浏览器启动前核验5001唯一listener PID、进程开始时间、精确仓库target/debug/nola可执行文件及实际NOLA_DB_PATH。相对进程路径以该PID的cwd解析，不以脚本cwd猜测。临时DB拒绝指向外部的symlink；环境从无环境argv精确前缀后提取，不让argv中的同名参数冒充环境；原始环境/stderr不输出。每个获准POST再次检查同PID/DB，失配直接abort，无fallback。只运行guard3/3和当前listener只读核验（verified），未重跑UI、未新建会话。脚本中不存在重复的同一inputValue断言。

## 原生缩放边界与费用确认／取消闭环

新增 `web/playwright_runtime/ui_probe_confirmation_audit.cjs`，复用listener归属guard、独立临时Chrome profile。实际执行session90679退出0，日志 `/tmp/nola-probe-ui-audit.log`；证据目录 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-probe-ui.52vXyG` 含result.json与已目检的probe-cancelled.png。所有POST均被fixture响应或拒绝，连接目录也是纯合成数据；未发送任何Host mutation、OAuth、探测或模型请求。

真实键盘Meta+Equal五次前后均为innerWidth1440、devicePixelRatio1：此headless环境未能触发浏览器原生缩放。立即停止，不写Chrome偏好、不使用CSSzoom/CDP缩放代替，因此原生200%仍未验证，不能宣称三页缩放验收。

费用确认闭环通过：未勾选费用确认时按钮disabled；勾选后才能发出一次fixture请求，精确携带account及revision、credential binding、connection revision、model、protocol、max_requests=1和成本确认。请求挂起时取消绑定同一request ID；随后返回409取消回执，不生成verified证据、不自动重试，清空确认并恢复disabled。blocked=[]、pageerror=[]。取消成功即时显示取消提示，原请求终态随后显示保守的“Not confirmed…可能计费、未重试”，不会误称退款或能力通过。此项是实际页面+隔离协议fixture证据，不是付费provider端到端验收；未重复全套430测试。

## 会话三入口右侧面板

按新需求实现标题最右checklist／folder／sliders：进度、资料、会话设置。默认收起，同入口再次点击关闭，其他入口切同一面板；窄屏只一个原生dialog overlay，内保留同三入口、关闭、Escape及焦点返回。不是三套消息视图；实际canonical回答DOM节点与未发送草稿在切面板前后保持。旧四tab组件与旧专用测试已删除，无双导航兼容层。

技能采用 `toolbars.md › Item groupings` 的trailing inspector入口、`split-views.md › Desktop` 的可隐藏面板，以及cross-platform的ARIA/dialog翻译；不将Apple顶级导航强套Web。沿用水墨tokens、36px桌面/44px窄屏控件，单层内容分区，没有百分比动画或正文提取todo。

真实数据：GET/HEAD `/api/chat/conversations/:id/plan` 先核对同Host数据库会话存在，再通过现有 `session_plan_record → ConversationPlanCommand::Get`，响应 `{conversationId,authoritative:true,plan}`。明确plan=null空态，失败固定503 `conversation_plan_unavailable`，回执owner不匹配拒绝；没有写操作。todos状态只来自Core，取消不算完成，verification不混入任务状态。已有goal projection requirements与traces summary分别呈现，不把没有Goal的会话假装没有计划。资料复用文件、数据源、知识及网页引用；设置复用配置、上下文/缓存用量。SessionSettings仍服务composer原有权限/推理quick controls，不因面板关闭丢草稿；其策略ACK轮询只在busy/activeRun存在时运行，资源与进度首次打开才挂载。

owner安全：切会话关闭面板、清访问状态；进度请求以conversation/generation/request与Abort核验，清空旧快照后加载，迟到回执不写新owner。主消息流不重挂。

最终验证：Svelte check 0 errors/0 warnings、Node431/431、Vite build成功（仍有既有大chunk提示）；日志 `/tmp/nola-chat-inspector-check.log`、`/tmp/nola-chat-inspector-full-tests.log`、`/tmp/nola-chat-inspector-build.log`。Rust新增read协议1/1，`cargo build -p nola_cli --bin nola --offline` exit0，日志 `/tmp/nola-web-plan-route-test.log`、`/tmp/nola-web-inspector-build.log`；保留linker eh_frame warning，不自行重启正式服务。

`chat_inspector_audit.cjs` 自有随机loopback静态server与全API fixture，不连接5001、不使用正式数据库。最终session48846 exit0；`/tmp/nola-inspector-browser.log`；证据 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-inspector-ui.pn3MAw`。完整canonical回答、配置与plan fixture通过四闭环：三入口默认/切换/关闭；实际回答DOM及草稿不变；390px单overlay内切换/再次关闭/Escape焦点恢复；切会话清空并拒绝旧goal迟到回执。1440/390截图已目检，页面没有由history/config fixture缺字段导致的错误提示。早期MjmYTY错误页截图只作局部测试，不能替代本次正常页面证据。errors=[]、blocked=[]；无模型请求。

## 2026-10-26：浏览器冷恢复身份与取消停止见证

浏览器 client identity 和 source submission intent 统一使用 Host 范围的持久存储；首次身份创建通过浏览器锁串行化，多标签页复用同一身份。不可读写时明确暂停恢复，不生成临时身份，不迁移或双读旧 sessionStorage。Host 对无法确认的原运行控制身份返回 `run_control_unconfirmed`，页面立即禁止发送、审批及控制，提供只读重试；观察开始先锁定提交，只有 Core 的权威空运行或终态才能恢复。

真实 Chrome 创建的 Run `a7abd4dd-8500-4c96-8faf-91d5ea2cfdea` 验证了关闭标签页后同身份/同 Run/停止入口恢复，以及空白 browser context 不能取得原运行控制权。后续测试搬运 profile 时遗漏 IndexedDB，造成草稿 ticket 缺失；这是测试数据不完整，不是正常退出 Chrome 删除了 IndexedDB。修复后，合法已确认的 source receipt 不再被草稿 bookkeeping 失败阻断观察或停止；保留草稿与存储警告，不制造 ticket、不擅自消费草稿。

原 Run 首次停止返回202但没有终态。根因是同步权限等待返回时未保存物理退出事实，随后同 Run 的 `begin_turn` 又清除了本地停止见证。现在同步 handler 返回且等待登记已移除后保留退出事实，EOF 本身不生成权限决定或取消 ACK；同一保留 Host/Run 恢复时保留见证，新 Run 清除，任何同 effect 新派发仍清除旧见证。使用恢复后的见证前，核对 Core 当前取消控制以及目标 request、attempt、generation、owner、token、expiry；尚未结清的旧 started attempt 不能被后继见证覆盖。

在备份并正常退出隔离 Host 的旧进程后，用同 epoch、同数据库和原合法浏览器回执收尾：该 Run 已有 `sealed/cancelled`、`unknown_effect_refs=[]`。最终两次完整浏览器关闭重开均通过：Core 权威无 active Run、原 journal terminal seal 可读、停止按钮隐藏，输入未发送草稿后发送按钮可用。证据为 `/tmp/nola-real-browser-cold-identity/report.json`、`cancelled-cold-1.png`。报告保留过早动画断言与冷恢复已无 active Run 时不应继续要求旧当前 Run 的测试修正说明。Web520项测试通过；原权限 driver 回归与真实 SQLite 停止 fence 正反例通过。

### External 本地停止见证：共享校验边界

后续审阅发现，恢复后的本地停止见证错误复用了 `InspectCancellationProcessOwners` 整条 OS 死亡证明路径。该接口有意拒绝 External 类型，因为宿主进程死亡不等于外部操作停止；这一类型过滤不应挡住已由可信 Host 实际观察到的本地停止事实。

现从 Core 现有逻辑提取 `validate_cancellation_control_binding`：只校验当前取消控制及完整 committed delivery fence，不证明目标已停止。OS 死亡路径复用它后仍执行原有目标类型和进程死亡检查。CLI 本地已停止路径读取既有 `LoadRunCancellationDeliveryFacts`，复用共享校验，再检查其保留的目标完整 fence 和未结清历史 attempt。没有新 wire、状态系统、旧格式兼容或自动完成规则。未知外部效果仍必须独立核实，不能用 registry 为空或父进程退出冒充停止。

生产修改已通过 `cargo check -p nola_cli --lib`，日志 `/tmp/nola-external-stop-shared-check.log`。原有 Core typed OS owner 取消回归1/1通过（`/tmp/nola-external-stop-core-owner-test.log`），统一 CLI 构建通过（`/tmp/nola-external-stop-final-build.log`）。现有真实 OS OwnedShellChild 三项通过（`/tmp/nola-cli-owned-group-os-tests.log`），覆盖 kill/wait、leader 退出后残留进程组及 Unknown 业务结果不提升。真实 SQLite `retained_stop_` 三项通过（`/tmp/nola-external-local-stop-complete-tests.log`）：Core 实际 External ExecuteTool 链的本地入口接受当前完整 fence，OS 死亡入口仍拒绝 External，外来 control、五个 fence 字段变化、重绑与未结清旧 attempt 均拒绝。该层验证身份合同，不冒称实际工具已物理停止。新增真实 SQLite＋driver＋OS 完整链1/1通过（`/tmp/nola-cli-real-store-process-stop.log`）：实际 bash 子进程启动后定向停止，生产 kill/wait 返回强制要求的 typed local_stop，PID/effect/request 匹配且 waitpid 确认回收；丢弃业务响应后提交 Interrupt，同 Run 恢复经默认生产校验与 SQLite 接受 CancellationConfirmed，跨 Run 则拒绝。模型完成事件为测试注入，不含模型请求或网络，也未冒称 Service IPC 验证；早期失败来自 fixture 漏掉生产 pump 的 bind_file_read_run 边界，已补齐而未放宽生产校验。同一已编译产物相关15/15回归通过（`/tmp/nola-cli-external-stop-final-regression.log`），覆盖7项本地停止合同、3项 Store/fence、1项真实 SQLite＋OS 同 Run 恢复取消、1项权限等待恢复、3项真实 OS 进程组。此完整链是 warm retained Host 恢复，不是进程死亡后重建。此前 External 本地已停止见证误受 OS 类型过滤的限制已关闭；缺少独立停止证据的 External 仍不确认。不重复声称这次无 wire/UI 变更已重新执行全平台真实模型测试。

### 2026-10-26 双子任务真实执行：取消控制被父模型批次阻塞（待真实复验）

- 隔离 Host 5157 的真实 Chrome 点击创建两个子任务，两个子 Run 均实际准入；对 B 点击取消，对 A 发送指导后故意丢失已提交的 HTTP 回应，再关闭整个浏览器并以原持久 profile 重开、点击确认原指令结果。A 的唯一原 message ID 最终 `delivered`，没有重复发送指导。证据：`/tmp/nola-real-multiagent-cold/report.json`。
- 本轮**失败**：A、B 均 `sealed/answered`，B 未取消，父 Run 未终结。不能以两个模型产出或指导成功代替多任务控制闭环通过。
- 根因已由实际 outbox/attempt 证明：取消提交已生成 `CancelTask` effect 30（30 秒执行期限），但共享执行泵等待当前父模型批次返回，126 秒后才领取。Host 的 `ExecutionRejected(deadline_exceeded)` 又被 reducer 当作无 tool call 的普通失败返回错误，事务回滚，取消 effect 无法收尾，后续效果被阻塞。
- 修复集中在共享 Core：窄控制 claim 从当前已提交 outbox 选择任务取消并复用既有租约 claim；共享 runtime-driver 在物理批次进行时及时执行 Application 控制。执行入口校验当前 effect/request、完整已开始的 delivery fence，以及 source conversation/Core session/Run/Turn 归属，再调用现有 RuntimeTask 取消请求。只提交停止意图，子 Run 的真正停止仍须原 worker 和不可变终态回执证明。CLI 删除原重复任务取消业务分支。
- 对实际 `CancelTask` 的过期/执行失败提交明确 `task_cancellation_failed` 和失败尝试收尾，不把子任务标为已取消，不使普通取消 effect 绕过停止证明。
- 定向验证已通过：共享执行泵 12/12（含长物理批次中控制及时派发）；真实 SQLite selective claim/启动与完整 fence/过期失败收尾 1/1；已完成子 Run 优先于晚取消 1/1；CLI check。日志分别为 `/tmp/nola-task-control-pump-all.log`、`/tmp/nola-task-control-store-test.log`、`/tmp/nola-task-control-terminal-race.log`、`/tmp/nola-task-control-final-check.log`。这些是确定性协议/执行泵验证，**新真实双任务模型复验尚未通过**。

#### 双任务修复后的真实复验（5159）

- `/tmp/nola-real-multiagent-control-fixed/report.json`：真正创建两个子 Run；B 的 `CancelTask` 在父模型仍执行时正常 claim/start/settle，B 最终 `sealed/cancelled` 且 `unknown_effect_refs=[]`；A 最终 `sealed/answered`；父 Run 最终 `sealed/answered`，真实汇总 A=125250、B 已取消且不重启。
- A 指导的真实 Host 202 已提交后被测试故意丢弃回应；整个 Chrome 关闭重开，使用原 profile 点击“确认原指令结果”，同一个 message ID 最终 `delivered`，没有第二条指导。随后冷开实际点击“打开子会话”，完整只读 journal 中看到 `MULTI_A_VERIFIED`，返回父会话后 Stop 消失，填入未发送草稿后 Send 可用。截图 `guided-child-complete.png`、`final-summary-cold.png` 已目视。
- 首次断言错误保留在 `first-assertion-report.json`：marker 只要求子 A 输出，却检查父汇总 DOM；随后又发现任务摘要恰好按 4096 字节截断，不能拿摘要替代完整结果。最终保留 marker 断言并通过真实任务入口读取完整 child journal，不降低内容核验。详情入口须等 journal 卡真实渲染后点击，未修改产品来绕过测试等待。
- 原 5157 失败现场保存并正常退出旧 Host 后，同库恢复原父 Run 为 `sealed/failed`（原模型请求已过期限），两个旧子任务仍为 answered；无重发取消、无新用户输入，不能把旧失败记成成功取消。证据 `original-cold-recovery.json`。
- 留下一个独立显示问题：child 的指导用户气泡显示在包含指导内容的完整 assistant 答复之后；同 Run 继续追加答案仍使用原 assistant ordinal。控制、内容和终态已正确，但该历史展示顺序尚需单独修复。少量迟到的 task progress 更新因租约过期被明确拒绝，不改写上述终态。

### macOS 本地任务指导：统一持久意图与 Core 回执（2026-10-26，验证进行中）

桌面本地指导原先经 stdin `task_guidance` 帧发送，接收方重新生成消息身份，断线重试不能证明是同一条指令。当前调用者已改为共享 `AppleTaskGuidanceOutbox`，固定完整意图及消息 ID；本地与 iOS 复用 `AppleTaskGuidanceCoreTransport`，先由 Core 查询精确父 Run 的指导回执，再仅在权威空回执时提交原意图。父 Run 与父 Turn 分开传递，不以当前任务展示名称或会话字符串猜测权限。恢复发生于精确 Run admission 后，未确认的指导保留并显示错误，不终止正在运行的任务。旧 desktop `task_guidance` 帧及分派已删除，合法任务取消帧保留。

已完成 Swift 解析、共享文件类型检查及 CLI check。Mac Test 配置的共享 outbox 与远程工具计划时钟契约定向构建正在执行，尚不把编译启动计为测试通过。另发现 source Store 与全局任务 Store 分离时，共享 CancelTask 执行不能直接查询 source 的全局任务行；该跨 Store 授权缺口正在独立修复，不能用本轮同库真实任务通过替代。

上述 Mac 本地指导定向验证现已完成：完整 Test 配置原生构建成功，`AppleTaskGuidanceOutboxTests` 5/5 通过（`/tmp/nola-mac-guidance-final-test.log`）；覆盖断线固定意图、原回执恢复、权威空回执重交、独立任务不互相饿死及提交前精确父 Run 校验。初始额外时钟测试过滤错类，日志明确显示该类执行 0 项，未计通过；改用正确 `MacAgentHostEventContractTests` 后复用同产物 test-without-building，远程工具计划结构/时钟契约 1/1 通过（`/tmp/nola-mac-remote-clock-final-test.log`）。这些是原生编译和传输契约测试，并非新的真实模型指导会话。

### 无保存父会话与全局子任务库：取消授权跨 Store（2026-10-26）

新增真实双库回归发现，父会话在内存 Store、子任务在全局持久 Store 时，直接在 source 查询全局 task 会失败；同库 `request_runtime_task_cancel` 的 changed 布尔也不能作为重复取消后的持久 `cancel_requested`。当前 Core 返回明确 Applied/Transfer 结果：同库仍在当前事务请求停止并返回持久意图；分库必须验证原 source 上已开始的完整控制 delivery fence，再导出原始任务 admission 与该控制绑定。全局 Store 通过现有可信 Host IPC 验证 immutable admission 一致后请求停止。普通 Application JSON 不能构造导入命令；Host 固定完整授权摘要作为命令回执 ID，丢响应沿既有回执协调，不能重新生成停止意图。任何接受只表示停止请求，子 Run 必须另行提交其真实终态。

同时修正原 source admission locator：后续仍为 queued 的 TaskUpdated 不能被误认成新的准入。定位必须落在真实同任务 TaskPromoted 的已提交事务范围，最后仍由原 bounded exporter 严格验证。

验证：真实内存 source + 独立 SQLite task Store 的精确授权、外来 fence、合法但不匹配的 admission、过期和重复导入 1/1 通过；同库 selective claim、过期失败可继续及重复停止返回持久意图 1/1 通过。对应 `/tmp/nola-split-task-control-test.log` 与 `/tmp/nola-same-store-task-control-test.log`。这些是实际 Core/SQLite 事务测试，无真实模型调用。服务锁等待过期回归另行记录最终结果，不能以入队前时间代替实际执行时刻。

服务锁等待过期回归最终 1/1 通过（`/tmp/nola-split-control-service-deadline.log`）：实际服务处理线程等待 Application mutex 至控制过期，取得锁后以新执行时间拒绝，不进入目标停止操作；命令原始参数及固定回执身份不变。该服务排队测试使用构造的有效授权元数据，只验证时钟边界；成功 source 导出与目标所有权由上述独立真实双库测试验证。当前改动 diff 检查通过，生产源码冻结待统一 CLI 构建。
