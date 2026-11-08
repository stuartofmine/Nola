> **阶段归档：2026-10-28。** 以下为当时的实施与审计记录，文中的“当前”“进行中”和执行顺序不再构成现行任务指令。归档不等于全部验收通过；结论见[归档摘要](README.md)，当前任务状态见[当前入口](../../当前任务/README.md)。

> **单版本开发约束：仅保留当前架构、接口、协议和数据格式；不新增兼容层或数据转换，不自动删除、重建或改写用户数据。**

# CLI UI 审查与优化

## 2026-10-23：恢复退出前的显示状态

此前“恢复”只重放近期历史并滚动到底部，不能视为恢复原界面。本轮增加 Host 本地 `CliViewState`，以规范数据库路径与会话 ID 分别散列隔离。仅保存未发送草稿、UTF-8 光标位置与阅读锚点；不保存第二份会话历史，不更改 Core Run/消息/权限状态。原有模型、权限与任务状态仍由 Core 恢复。

- 编辑与翻页时原子保存小型视图记录，内容未变不重复写盘；文件权限 0600、作用域目录 0700。关闭终端后可恢复未提交草稿和光标，而不只支持主动 `/exit`。
- 阅读锚点优先绑定显示源内容与源内位置，辅以可见内容定位，处理终端宽度变化产生的重新换行。重复内容保存出现次序，不能无条件选择第一处同名行。
- 历史恢复先完成布局再展示，不把旧消息逐段当流式输出重播。已结束会话不再启动执行输入监听器，避免恢复时出现第二个短暂输入框并吃掉快速输入。
- 格式不匹配、损坏记录、无效 UTF-8 光标明确拒绝并保留文件，本次会话停用该文件写入，不迁移、删除或自动覆盖。不同会话和不同数据库的草稿不共享。
- 锚点已不在当前加载历史范围时明确提示并保留草稿，不能宣称精确定位成功。当前历史加载仍有 96 条消息、160000 字符等限额；跨该范围的按锚点分页加载尚未实现。

实际验证使用真实会话数据库副本与产品 CLI，VT100 只解析真实输出，不伪造消息。`scripts/test_recorded_cli_lifecycle.py --exercise-view-restore --oracle <terminal_acceptance_oracle> [--view-columns 70]` 会输入中文草稿、将光标左移、PgUp、关闭测试自身 PTY、重开同一会话，并验证显示位置和插入字符落点。原用户进程与数据库未改动。

同宽恢复实际比较前 15 行画面和光标列；100→70 列恢复检查同一段可见内容、阅读模式与光标编辑。第一次窄屏测试暴露整行匹配失效，已改为跨换行内容匹配。后续快速输入测试暴露已完成会话仍启动执行输入监听器的问题，也已修复；这些失败不计为通过。

状态存储隔离/格式拒绝回归 1 项通过；原终端渲染回归 74 项通过。证据保留在 `/tmp/nola-cli-view-store-final-tests.log`、`/tmp/nola-cli-view-terminal-final-tests.log`，真实 PTY 的逐帧记录和 before/after 画面由脚本写入其输出目录。不声称恢复过期审批弹层或任意外部进程的运行状态；执行期间尚未交回普通编辑器的草稿、超出历史加载范围的锚点仍需另做覆盖与收敛。

最终产品二进制复测：`/tmp/nola-cli-view-100-verified.log` 与 `/tmp/nola-cli-view-70-verified.log` 均通过，包括草稿/光标/阅读定位、恢复后插入字符、菜单取消、退出提示、不创建空会话和不新增 Run。测试复制的 Host 凭据已删除；画面与逐帧证据保留在日志首行列出的私有临时目录。

## 2026-10-23：对照本地 Codex 的退出与恢复设计审查

参考范围为仓库 `参考资料/codex-main` 中的源码快照，不宣称代表最新发布版。以下保留审查时的问题与设计约束；本轮落地和验证见本节末尾，不能把设计清单当成全部通过。

### 已确认的结构问题

1. **中断与退出语义不一致。** Nola 输入编辑器的 Ctrl+C 清除草稿并返回 None，外层直接退出。Codex `tui/src/chatwidget/interaction.rs::on_ctrl_c` 先让当前弹层/输入区处理，再区分正在执行的工作和退出；`on_ctrl_d` 只在空输入且无弹层时参与退出。该快照的 `DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED` 是 false，不能将未启用的双击分支当作当前行为。
2. **会话选择取消会变成退出。** Nola `/resume` 通过 `return restart_interactive_session_restore(...)` 离开当前交互循环；选择器 Esc 返回 Ok，随后原循环也返回。应将选择器作为当前界面的临时状态，取消回到原会话和草稿，选定后再切换。Codex `OpenResumePicker` 使用应用事件处理，不靠嵌套交互循环表达切换。
3. **恢复失败直接终止界面。** Nola `run_interactive_prompt_with_recovery` 在恢复阶段使用 `?` 或直接 return，用户无法留在会话中查看状态、重试或选择其他会话。展示历史、附着运行时、继续尚未完成的工作应分开，失败保留可操作界面；不能在原 Run 未解决时悄悄提交新任务。
4. **恢复入口行为不一致。** `--resume` 进入恢复流程，空消息 `--session ID` 进入普通交互循环，仅恢复显示历史；退出提示却只写 `nola --resume`，没有唯一会话身份，且选择列表依赖启动目录。应统一选择后的会话附着流程，退出给出精确、正确转义、不因窄屏截断的恢复命令；工作目录采用明确规则。
5. **界面退出与运行停止缺少统一用户契约。** Nola 外层确有 supervisor shutdown，不能据 `/exit` 的 break 就断言完全没有清理；但交互层没有统一的“正在收尾 / 已停止 / 尚待确认”结果。Codex `app/event_dispatch.rs::handle_exit_mode` 明确区分 ShutdownFirst 与 Immediate，先尝试关闭当前线程，并设 2 秒 UI 等待预算。Nola 应借鉴退出与运行状态分离，而不能照搬超时作为任务已取消的证明。

### 实施顺序与不可绕过的边界

- 先建立单一交互会话循环与明确结果：返回原会话、选定会话、请求退出；去除恢复页面递归进入交互循环的控制方式，保存当前草稿和选中状态。
- 统一会话附着入口：指定 ID、最近会话、列表选择都进入同一流程。先展示历史，明确活跃 Run 状态；恢复失败留在界面。只有 Core 确认允许新输入时才发送新一轮。
- 区分“中断当前轮”和“退出 CLI”。退出停止接收新执行输入，向当前所有者发出收尾请求，显示进展并有有限界面等待；正常退出前记录精确恢复命令。超时保留真实 pending/unknown 状态，不能伪造 CancellationConfirmed，不能清库、改状态或靠杀用户进程达到通过。
- 后台任务、子 Agent 与会话的所有权由 Core/Host 生命周期契约处理，不能仅因父窗口关闭就宣称全部停止，也不能无提示遗留当前前台工作。

### 必须通过的真实 PTY 场景

带中文草稿按 Ctrl+C；空闲 Ctrl+D；执行中 Ctrl+C 与 `/exit`；打开 `/resume` 后 Esc 返回原草稿；切换会话再返回；从不同工作目录执行退出提示中的精确命令；模型/工具正在输出时退出并重启；审批等待时退出恢复；Host 无响应时界面能退出而 Store 仍如实 pending；连续恢复不重复执行副作用。每项核对最终 Core Run/投递状态与进程状态，不能只看终端文字或测试退出码。

### 本轮实施与实际验证

- 会话切换改成外层迭代循环；选择器返回取消/选定身份，不再递归启动交互循环。选择器在现有 alternate screen 内暂时绘制，返回时使原画面缓存失效并重绘；取消和选择失败保留当前交互。
- 指定 `--session` 与列表选择共用恢复流程；选定后恢复该会话配置。仅打开 `--resume` 不再初始化空会话。退出提示给出完整、shell 转义的 `--session ID --project PATH`，不按终端宽度截断。
- 带草稿 Ctrl+C 清空编辑内容并留在输入框；空输入 Ctrl+C/Ctrl+D 保持退出语义。执行中的 `/exit` 立即请求正常取消，并只把本地退出控制交回输入循环，不发送给 Agent。
- 恢复报错或没有完成时保留界面，并阻止普通消息及新 Goal 越过未解决 Run；可以重试选择、切换或退出。恢复返回 None 时再次核对 active Run，不能把未执行恢复当成允许新输入。
- CLI supervisor 的退出等待设为 2 秒；超时报告停止未确认，保留实际持久状态，不构造取消成功。该预算仅覆盖 supervisor shutdown 阶段，不宣称强制终止任意失去响应的前台外部调用；原生 Host 与 Web 服务不使用此 CLI 退出预算。

真实验证均在用户会话 Store 的隔离在线备份上执行，原数据库和原用户进程未改动。新增 `scripts/test_recorded_cli_lifecycle.py`，显式传入 source DB、credential source DB、binary、session、project；默认不发送模型请求，`--exercise-model-exit` 才调用真实模型。私有凭据副本权限受限，测试后删除。

- `/tmp/nola-cli-lifecycle-navigation-final.log`：指定 ID 附着、中文草稿 Ctrl+C、会话内 `/resume` Esc 返回、完整退出提示、独立选择器 Esc 不创建会话、没有新增 Run，均通过。
- `/tmp/nola-cli-lifecycle-final-pty.log`：真实模型进入 running 后 `/exit`，退出 0，Core 结果为 Cancelled，pending effect 为 0；不是伪造模型事件。
- `/tmp/nola-cli-failed-resume-final-pty.log`：第一个 CLI 执行真实模型时第二个 CLI 恢复同会话，显示 live owner 错误并保留输入界面；第二个输入保留为草稿而未创建 Run，两个 CLI 最终退出 0，原任务 sealed。
- `/tmp/nola-cli-lifecycle-pty.log`：实际列表选择重新附着通过。两个早期并发测试未等待真正 Run/编辑器就绪，分别超时；不计通过，最终测试以数据库 running 和输入帧就绪为前提。
- 定向 lib 测试：interaction 26 项、退出命令解析 1 项、conversation_invocation 10 项通过。配置测试首次继承用户模型配置导致 7 项失败；使用独立 `NOLA_HOME` 重跑 10/10，通过日志 `/tmp/nola-cli-lifecycle-config-tests-isolated.log`，不把环境污染隐藏为一次通过。
- `target/debug/nola` 已重新构建；仍有链接器既有 unwind section 大小警告。审批等待、外部工具无响应、不同 cwd 执行提示命令以及子 Agent 全部停止未在本轮真实测试中覆盖，不能标为已验收。

## 2026-10-23：真实历史故障复核，不能以组件测试替代验收

用户再次反馈 CLI 大量问题。此前组件、夹具、VT100 或离线 PTY 通过，只证明相应范围，不证明真实模型、用户会话恢复、权限和长任务可用。当前没有“CLI 全部通过”的结论。

本次读取实际 Store 的在线备份，保持原始 Run、effect、租约和进程身份。凭据存储独立于产品数据库，隔离副本需配套复制其 Host 私有库并保持 0700/0600；不得把未复制凭据导致的报错记为产品原始故障，不输出或提交凭据。原 Store 未修改，原 CLI 进程未终止。

### 已复现并修复：空闲取消投递进程永久阻塞恢复

- 会话 `20261021233903837428000-96063-0` 的 Run `c217a57e-9d65-4ac6-a369-11eaaaa1cbc4` 长期 running。取消 effect `571` 指向 `561`；原目标各次持有进程已退出，但取消投递持有进程 52242 仍存活。
- 该取消投递停留在 Leased，从未 ExecutionStarted，租约早已到期。此前 cancel-only 路径把它与实际执行目标一律要求“进程死亡”，交互进程仍开着便无法恢复。
- 用重新编译的产品 `target/debug/nola --resume --project /Users/gaojc/Desktop/Nola/cli/scripts`，在真实 PTY 选择原会话，修复前实际退出并显示 `Run cancellation remains pending: target owner 52242 is alive`。
- 现在仅允许**已过期且未开始执行的取消控制投递**重新认领。Application 在事务内比较完整观测事实并替换投递 fence；目标任务的停止证明、活跃会话租约检查不放宽。已开始执行或未到期的取消投递仍要求原持有者停止。
- 修复后重复同一路径，原进程仍存活，隔离副本中的 Run 经正常 Core 回执封存为 Cancelled，pending outbox 为 0，返回输入框。随后 `/allow` 立即打开权限面板，编号 2 可见，Esc 可退出。
- 随后在同一副本、同一会话使用既有模型发送无工具连通性追问，实际收到“CLI恢复后可以继续对话。”，新 Run 正常完成，进程退出 0。这是模型真实调用，不是预设模型响应。

验证证据：`/tmp/nola-cli-real-audit-build.log`；`/tmp/nola-cli-real-audit-cancellation-tests.log`（lib 定向 12 通过、2 个需真实 Store 的用例另跑）；`/tmp/nola-cli-real-audit-recorded-tests.log`；`/tmp/nola-cli-real-audit-followup.jsonl` 与 `...followup.stderr`；只读展示结果 `/tmp/nola-cli-recorded-after.json`。本次 `--bin nola` 误选产生的 0 测试不计为通过证据，实际测试属于 lib target。

### 历史反馈仍须逐项验收

| 路径 | 本次结论 |
|---|---|
| 原会话取消、重启恢复、恢复后真实模型续聊 | 已完成上述故障的真实副本验证；不是直接修写用户 Store |
| `/allow` 与编号 2 | 恢复后空闲输入路径已实际操作；执行中即时命令仍待真实长任务验证 |
| Code Memory 初始化、自然追问与重复搜索 | 既有记录保留；本次未在用户实际仓库重新完成真实模型验证 |
| 新会话、切换模型、权限复用、长任务、多 Agent、流式思考 | 不能继承上述恢复测试结果，仍须独立端到端验收 |

后续验收必须记录产品二进制、真实入口、模型/工具是否真实、最终持久状态及未验证项。不得仅根据“进程退出 0”“测试数量”或没有报错关闭用户问题。

2026-10-19。终端共享语义，不复制 GUI 的像素布局。

## 已确认的问题与修改

- High：`main_parts/bootstrap.rs` 的正文固定 ANSI 255 白色、输入框固定 236 深灰色，不能适应用户浅色/高对比终端；标签同样硬编码灰色。已改为终端默认前景/背景，取消全行强制深色底。主次通过布局和文字表达，不压暗必要说明。
- 选中会话由固定深绿背景改为终端反显，保留原有选择标记；不只靠颜色辨认选择状态。正文重置粗体，避免输入提示的强调泄漏到整条输入。
- 添加 vt100 回归验证正文默认色、选中反显及默认色继承。定向测试 1/1 通过，退出0；链接器报告既有 `__eh_frame` 超16MB警告，不影响本项结果，也不声明零警告。

## 保留与边界

保留现有唯一输入区、滚动所有权、取消和授权回执协议。此轮未改执行状态或数据模型；无颜色模式继续由现有 NO_COLOR/TERM=dumb 控制。水墨强调色尚保留，不能据正文配色修复声称任意终端色板对比度全部合格。

待继续核对：窄屏菜单信息优先级、中文分页说明、会话选择和模型选择完整操作，以及真实 PTY resize。此前 PTY 证据仅证明当时交互，不代替本轮修改后验证。

最终终端UI组：`cargo test -p nola_cli --lib terminal_ui::tests --quiet` 71/71通过，退出0，覆盖既有布局/滚动/输入回归及新增色板测试。此前误选`--bin nola`运行0项，不计通过证据；已改用实际拥有测试的lib target。链接器仍有上述unwind大小警告。vt100测试不等于真实终端外观或人工交互验收。

## 最新色板真实离线 PTY 验收

执行 `python3 scripts/test_offline_cli_ui_terminal.py --binary target/debug/nola --service target/debug/nola-service --oracle target/debug/examples/terminal_acceptance_oracle`，最终exit0，日志 `/tmp/nola-offline-cli-ui-pty-final.log`（首行给隔离证据目录）。产品binary时间17:20:54晚于色板源17:18:06；SHA256 `de1e1430c94e8616b9ef8d272811a49fc0747fefc2890f70eb5698756efcda85`。实际ANSI验证反显选中且无旧236输入底色，不只比较mtime。

仅用临时数据库，通过现行Core导入两条明确标记的离线历史；未读取用户配置/凭据，未启动模型fixture或访问LAN。真实TTY进入会话选择，40列重绘后下箭头+Enter选择第二条，恢复到原会话；20×40、30×120、24×60 resize后每个稳定帧仅一个输入提示，中文未提交草稿仅一份。退出前Ctrl-U清草稿并确认空输入，再由既有身份校验cleanup收口；独立只读SQL确认Run数为0。保留raw ANSI、帧、动作和进程回收证据。

验证器只扩展纯VT Resize操作（尺寸有界），不改变产品。会话选择器是直接TTY输出，检查原始字节及真实选择结果，不伪装为同步提交帧。此前失败包括fixture空历史被排除、归档ordinal、macOS路径canonical差异和继承NO_COLOR；全部保留。r6检查时发现共享cleanup把/exit拼到草稿，权限缺配置阻断了执行；修为先清草稿后，r7及最终轮均Run=0，以最终轮为准。

边界：验证真实PTY进程/尺寸/输入行为与ANSI语义，不是人工真实Terminal.app浅深背景像素对比；未验证模型流式输出、执行授权或所有终端模拟器。没有重跑Android或扩展聊天执行架构。


## 执行中草稿持久化（2026-10-23）

空闲编辑器与执行监听器共享同一个 ViewStore 句柄及写入去重状态。监听器启动时捕获所属会话句柄，不从后续切换的会话取路径；只有输入、粘贴、光标、滚动或尺寸变化时保存，不在空轮询中刷盘。新会话首次提交生成 ID 后也绑定保存位置。保存失败明确提示，原有格式拒绝和数据隔离保持不变。

验证：共享句柄/会话切换/格式拒绝专项 1/1，通过 `/tmp/nola-cli-active-draft-tests-final.log`；无默认功能检查通过 `/tmp/nola-cli-active-draft-no-default.log`。真实模型与真实 PTY（隔离旧会话副本）验证运行中的中文草稿及 UTF-8 光标落盘、清空输入同步、`/exit` cancelled seal 且无 pending 投递，结果 `/tmp/nola-cli-active-draft-real-pty-final.log`。首轮脚本未持续读取 PTY 输出导致背压超时，修正观测循环后重跑通过，旧失败保留。

额外强制关闭重开测试：运行中草稿恢复已出现，但整组**失败**，不能记为通过。`--exercise-active-draft-restart` 在重开后取消时报 `cancellation target stop is unproven in this Host; durable owner reconciliation is required`，Run 仍 running、未生成 result。日志 `/tmp/nola-cli-active-draft-restart-pty.log`，隔离现场 `nola-lifecycle-pty-aqetqb2v`，Run `09bc9b32-e894-4b07-9688-fab94ec1db65`。脚本仍保留要求 cancelled seal 的失败断言，没有放宽验收。该恢复 owner/stop 证明链为后续必须修复项；不能把草稿显示成功表述成整个执行恢复成功。

超出最近 96 条消息的阅读位置恢复仍未完成：现有书签为显示内容锚点，需接入具体消息身份及现有 Core 历史分页，不能通过无限加载历史代替。


## 强制关闭后立即取消的停止证明修复（2026-10-23）

上节失败已复现并定位：模型请求的原执行进程退出，恢复将其 attempt 标为 interrupted；用户在新 Host 重放前按 `/exit`，Interrupt 退休目标 outbox，但新 Host 只查自身 quiescent 缓存，缺少旧执行的停止证明。

正常 Host 的取消分支现在复用 Application `LoadRunCancellationDeliveryFacts`：核对当前 conversation/Run、assistant、control 请求及当前进程持有的投递 fence，沿用 `supports_dead_owner_confirmation` 限制，检查全部历史 target attempt 的本机 owner 已死，再次读取事实确认未变，最后仍经过原有 fenced start/response 事务。不把租约到期、空注册表或 UI 退出当作停止证明；活 owner、未知 owner 和已启动外部工具继续拒绝。没有改写用户数据库或新增迁移。

- 取消专项 13 通过、2 个 opt-in 现场用例跳过；新增活/死目标 owner、错误 Run、错误 request ID、错误 token 回归，日志 `/tmp/nola-cli-recovered-cancel-tests.log`。
- 新构建真实模型＋PTY 重跑原失败路径成功：运行中中文草稿及光标落盘 → SIGHUP 关闭测试进程 → 同会话重开恢复草稿 → `/exit` → 原 Run cancelled seal、pending 投递 0、无重复 Run。日志 `/tmp/nola-cli-recovered-cancel-real-pty.log`，隔离证据目录 `nola-lifecycle-pty-fhutu09o`。旧失败记录保留，不改变失败断言。
- 这是模型请求的断开/取消恢复验证，不扩大为任意外部进程或多 Agent 的停止保证。超出最近 96 条历史的书签分页仍待完成。

补充收尾验证：Host 启动/停止屏障在隔离 `NOLA_HOME` 下 24/24，通过 `/tmp/nola-cli-recovered-cancel-barrier-isolated.log`。首轮 4 项因继承本机模型目录选择与测试显式端点冲突而失败，日志 `/tmp/nola-cli-recovered-cancel-barrier.log` 保留。无默认功能编译通过 `/tmp/nola-cli-recovered-cancel-no-default.log`；既有链接器 `__eh_frame` 大小警告仍存在。真实 PTY 产品二进制 SHA256 `38b5f8552b8e9a631c7566593e1d769e27aeca533119ce39009bf58b2beed5a6`，封口 Run `0d0de713-cbed-4163-adb8-48fc5333f138`。


## Harness 执行链与验收链审计（2026-10-23）

范围：CLI/Core Host 的执行输入、持久队列、任务通知、冷恢复/取消、信号、终端生命周期；Web 权限恢复的后继输入；Python PTY/进程 supervisor/公开验收器。是有范围的审计，不是所有平台、工具和多 Agent 全量通过声明。

### 本轮确认并修复

1. 删除 `persist_boundary_user_input` / `take_pending_boundary_user_inputs` 旧交接：前者的调用者已只剩测试，后者仍在生产完成/恢复路径中，按内容摘要认领后先 ACK 再返回普通字符串。这条路径不保留输入的持久交接身份，也不代表当前客户端通过 `boundary_input_intent` 的准入行为。现只消费任务控制通知；用户输入保留在 Application 队列，由 Core 的准入事务处理。测试改用真实客户端的准入 API，验证通知消费后输入仍在、释放/重新认领后 ID 与 payload 不变。未增加兼容转换。
2. PTY 验收使用备份完成后的固定基线，结束时不再读取仍可能变化的用户源库；比较 Conversation/Run 身份集合，而非仅数量；按本次新增 Run 的精确 ID 核验 seal 和 pending，不以时间排序猜测目标。
3. 复制测试凭据、查配置和打开 binary 全部纳入 `try/finally`；准备阶段失败也清除本次隔离凭据副本。新增故障注入检查缺 binary/缺凭据，确认不报告模型成功、原凭据不改写且临时副本删除。
4. 无论通过或失败都写 result.json，保留已完成步骤、失败类型、固定基线与二进制摘要。等待 Run 入库时持续读取 PTY，避免测试自身输出背压。主启动/重开/选择器明确中文语言，避免继承其他语言导致断言误判。

### 本轮证据

- Rust typed queue 3/3，执行输入/命令交互 26/26：`/tmp/nola-harness-audit-{queue,input}.log`。
- Web 权限恢复实际 HTTP 路由与确定性模型夹具 2/2：`/tmp/nola-harness-audit-web-resume.log`，覆盖原请求批准只执行一次、后继输入不被前一远端 client 接管。不是线上真实模型测试。
- Python 失败清理 2/2、进程 supervisor 3/3、公开验收器 7/7：`/tmp/nola-harness-audit-{failure,supervisor,acceptance}.log`。首次使用模块名运行碰到环境中同名 scripts 包，未收集测试；改用文件目录 discovery 后结果如上，不计首次导入失败为产品问题。
- 新产品二进制 + 实际 OS 信号 + HTTP 夹具 3/3，覆盖 SIGINT/SIGTERM 正常取消、SIGKILL 不伪造 seal：`/tmp/nola-harness-audit-signals.log`。
- 新产品二进制 + HTTP 夹具中英文正文合同 2/2：单次模型响应、不因解释命令输出而额外采样、公开正文与 Store 一致，`/tmp/nola-harness-audit-answer.log`。
- 真实模型 + PTY + 固定隔离数据库：运行中草稿/光标、SIGHUP、重开、取消、精确 Run seal 与无重复 Run 全部通过；`/tmp/nola-harness-audit-real-pty.log`，证据根 `nola-lifecycle-pty-ucxy9zzu`，Run `bde71615-d099-42c3-a4d5-0cb829338d2d`。binary SHA256 `d8fb356c60135f57f987e56c751b101ffb090287e6a00ef42965680ec8a5295b`。

仍需独立验收：超出历史窗口的阅读锚点、跨会话待发送输入、多 Agent 同时取消及子进程副作用停止、远程 Host 网络故障、Apple/Android 对同一恢复事实的行为。上述本轮通过项不能替代这些场景，也不能证明整个 harness 无问题。


### 会话输入归属复核（2026-10-23）

确认并修复进程全局 composer 让不同会话共享草稿和本地待发送队列的问题。现在按数据库位置与会话 ID 选择输入状态；执行监听器与审批回调持有原会话状态，不随前台切换。新会话首次取得 ID 时保留匿名编辑器，`/new` 清除匿名归属。执行失败没有返回草稿交接时，编辑器从当前会话状态恢复，避免用空字符串覆盖输入。此修改不改变持久化格式，也不声称进程内队列具备崩溃持久性。

- 输入模块 27/27 通过，包括跨会话、跨数据库同名会话、晚到审批回调、切回原会话及匿名输入绑定；`/tmp/nola-composer-isolation-tests.log`。这是实现级回归，跨会话真实 PTY 验收仍待补齐。
- 构建与无默认功能检查通过：`/tmp/nola-composer-isolation-build-final.log`、`/tmp/nola-composer-no-default.log`。
- 新二进制真实模型 PTY 恢复/退出通过：`/tmp/nola-composer-real-pty.log`，证据根 `nola-lifecycle-pty-gtbx4ufn`，精确 Run `5efd542d-cf99-4bdf-a791-bee47ded510f`，SHA256 `ebf4d08ceba2147bf6a283d518b54f67fe583c3774a533f91fad983d5e9a7232`。验证运行中草稿与光标落盘、关闭终端后恢复、退出确认取消、未新增会话。
- Core 后代优先取消状态回归 1/1：`/tmp/nola-cancel-tree-tests.log`，不代替实际子进程停止验证。
- 子代理真实验收脚本改为读取当前 `connections/default_model`，只使用所选连接明确声明的能力和凭据环境变量；运行在隔离 Home/Store，结束清理本次凭据副本。配置及取消回执 oracle 6/6 通过。真实模型结果另行记录，不以这些脚本单测替代。

- 真实 LAN 模型三轮验收通过：单次派生子代理实际启动 shell → 原会话取消并取得无未知副作用引用的终态回执 → 外来会话 `read_task`/`stop_task` 均实际拒绝 → 新进程回到原会话查询、不重新执行。启动标记存在、结束标记不存在，唯一任务/attempt/terminal receipt 在后两轮不变。证据根 `nola-subagent-lifecycle-etn2oi8w`，task `task-187db74b05f5`；`/tmp/nola-subagent-current-real.log`。随后使用增强后的验收器重新验证已记录三轮，额外核对 Store Run/result/seal/event 一致性并生成 `result.json`：`/tmp/nola-subagent-current-revalidate.log`。复核没有再次请求模型。
- 本轮未覆盖多个子代理并发取消、运行中 Host 崩溃、远程 Host 断网，也未补齐 >96 条历史锚点和跨会话真实终端输入隔离验收；不得将单子代理成功外推为全部通过。


### 跨会话真实终端补验（2026-10-23）

- 修正离线 PTY 夹具的旧模型配置和缺失会话配置：通过 `model apply-config` 建立当前模型目录，再用 Application 的配置 patch 选模型和权限。之前失败现场 `nola-offline-ui-pty-7lnfrcvy` 明确报 `Conversation permission configuration missing`，不是有效的交互验收。
- 删除该场景里已经与当前渲染器及其颜色测试相矛盾的“不能出现 236 背景色”历史断言；本轮不宣称通过调色验收。对应失败现场 `nola-offline-ui-pty-l5910u6u` 保留。
- 两个会话的草稿均通过真实键盘输入建立，未注入 ViewStore 文件；分别关闭终端、重开、实际插字核对 UTF-8 光标，再在同一进程中通过 `/resume` 往返切换。验证目标草稿恢复、无其他会话草稿残留、显式清空不复活、三种终端尺寸、原会话身份不变且 Run 数仍为 0。没有模拟或请求模型。
- 通过证据：`/tmp/nola-offline-current-pty-final.log`，根 `nola-offline-ui-pty-9v7og8fw`，二进制 SHA256 `ebf4d08ceba2147bf6a283d518b54f67fe583c3774a533f91fad983d5e9a7232`。清理失败也写进 evidence 并判失败，不再遗漏结果或跳过后续资源清理。
- 当前二进制 SIGINT / SIGTERM / SIGKILL 的实际进程 + HTTP 夹具回归 3/3 通过，`/tmp/nola-current-signal-audit.log`；不是线上模型测试。
- 这轮补齐草稿与光标的跨会话真实终端证据；执行中排队输入与晚到审批回调目前仍是实现级回归，不能自动扩展为该部分 PTY 验收通过。


### 并发子代理契约与验收（2026-10-23）

当前 Store 的准入规则是同项目只能存在一个可写/可执行命令的活动子代理；`read_only` 任务可与其并发。`spawn_agent` 的模型可见说明原来遗漏了这个约束，现已补充，包括冲突存在时重试不会解决，以及应等待原任务结束或显式使用只读子代理。没有放宽 Store 的写隔离。

首次真实测试错误地要求两个可写子代理并发，第二次派生被拒绝，模型随后重试并补开替代任务。验收按确切任务数量/执行身份判失败，未把顺序执行当作并发通过。失败根 `nola-subagent-lifecycle-var5ilkt`，`/tmp/nola-two-agents-real.log`；最终三个临时任务都已取消并 sealed，原证据未改写。后续场景改为一个执行隔离临时 shell 的 general 和一个沿文件链读取的 read_only explore。

新增独立观察器：必须在同一采样中看到两项任务 running、尚未请求取消且外部命令的启动文件已经存在，记录任务身份；最终检查取消回执、单次执行 attempt、无结束文件，并复核外来会话拒绝和冷查询不重派。观察器反例及原 oracle 共 7/7 通过：`/tmp/nola-concurrent-oracle-tests.log`。单测不能代替真实并发结果。构建通过：`/tmp/nola-spawn-contract-build.log`。

真实合法组合三轮通过：`/tmp/nola-two-agents-supported-real.log`，证据根 `nola-subagent-lifecycle-g1tayjdn`，任务 `task-8cf6877b44de` / `task-a731a376690d`。两任务同时 running 且临时 shell 已启动的采样保存在 `turn-1/concurrent-running.json`；两个取消回执均无未知副作用引用，唯一 attempt 未增加，shell 没有写出结束文件。外来会话对每项任务的 read/stop 均拒绝，原会话冷查询不重派。用最终验收器重验已记录三轮通过：`/tmp/nola-two-agents-supported-revalidate.log`，没有重复请求模型。

二进制 SHA256：`47ab064f1d90985f966d6bb8ca5be0255cc9b520744ae066528116326d01a4e5`。本轮是一个可写、一个只读任务的并发取消，不代表两个可写任务并发被支持，也不证明父 Host 崩溃自动收拢全部独立子任务。运行中 Host 崩溃、远程网络故障、超过 96 条历史锚点和执行期间跨会话排队输入仍待单独验收。


### 模型流式输出期间 Host 硬崩溃（2026-10-23）

通过实际 CLI + 隔离 Store + 受控 HTTP 流，在正文前缀已写入 journal 后发送 SIGKILL，再用真实 PTY 重开同一会话。发现并修复两个入口问题：

1. 参数解析在读取保存的会话配置之前就要求全局模型端点，导致无全局配置文件时恢复卡在配置向导。现在仅对保存会话的 `--session` / `--continue` / `--resume` 推迟模型校验，由会话恢复负责配置；新建及 `--no-save` 调用仍要求有效模型。失败现场 `nola-hard-crash-bjcgfk_b` 保留。
2. CLI 前台恢复和 Goal 待运行恢复使用新进程默认参数核对旧 Run，导致原 `--no-tools` 等执行设置丢失，误报 frozen invocation changed。现在复用已有 `restore_exact_core_agent_run_config`，按原 Run 的描述符重建并继续校验摘要、凭据、owner 和准确 Run ID；冷取消仍先走无需 provider 的确认路径。无活动 Run 的持久排队输入继续使用原准入流程。失败现场 `nola-hard-crash-hn64l2xs` 保留。

验收观察到旧流式 Part 被清空但身份保留，因此 oracle 按显示顺序拼接正文，而不是错误地要求只有一个 Part；仍要求全部公开正文精确等于响应，不能残留重复前缀。这个 oracle 修正前的失败 `nola-hard-crash-7ldoq9gl` 也保留。

通过证据：

- 当前配置解析隔离套件 69/69：`/tmp/nola-resume-config-suite-isolated.log`。首次未隔离全局配置时有 26 项受用户连接目录影响失败，记录在 `/tmp/nola-resume-config-suite.log`；未按产品失败统计，也未计为通过。
- 实际 SIGKILL + 受控 HTTP + PTY：`/tmp/nola-hard-crash-final-contract.log`，证据根 `nola-hard-crash-cg5tkxnq`。只有原来的一个 Run、一条用户消息和一条助手消息；原请求重试一次，共两个物理请求，冻结提示词、工具暴露及模型一致；最终正文不重复，未创建替代全局配置。不是线上模型质量测试。
- 真实模型 + SIGKILL + PTY：`/tmp/nola-real-hard-crash-pty.log`，证据根 `nola-lifecycle-pty-o9fotsud`。运行中草稿和 UTF-8 光标落盘、硬杀、重开恢复、退出确认取消原 Run，通过；原基线没有新增会话，精确 Run 为 `8b6a5e98-4d53-4448-83af-233196c4aefe`。本轮二进制 SHA256 `ab07653766632ab8bc31e5860078056f628de373f36564fe60291721fb185688`。
- 生命周期脚本失败清理回归 2/2：`/tmp/nola-hard-crash-pty-oracle.log`。新增 `--restart-signal kill`，必须与活动草稿重启场景一起使用，并确认真实 SIGKILL 退出状态。

本轮只覆盖模型请求中的硬崩溃，不证明外部命令/子进程执行中崩溃后能自动确认停止，也不证明任意外部副作用能够安全重放。远程网络故障、执行中跨会话排队输入以及超过 96 条历史的阅读锚点仍待验收。

恢复/权限入口套件的第一次默认线程栈运行在深层集成夹具中 stack overflow（`/tmp/nola-frozen-recovery-suite.log`）。按已有 16 MiB 测试栈条件重跑 23/23 通过后，将该夹具的栈预算改为显式、局部的 16 MiB worker，并传回原 panic，不再依赖全局 `RUST_MIN_STACK`。移除该环境变量重跑仍为 23/23：`/tmp/nola-frozen-recovery-suite-default-fixed.log`。产品线程栈未修改，上述实际 CLI / PTY 崩溃验收也没有设置额外栈。无默认功能编译检查通过：`/tmp/nola-frozen-recovery-no-default.log`。这些记录不把默认栈失败伪装成首次通过。

### 外部命令执行期间 Host 硬崩溃（2026-10-23）

新增 `scripts/tests/test_cli_external_crash_recovery.py`：实际 CLI 启动 bash、确认子进程 PPID 属于该 Host，再仅 SIGKILL Host，用真实 PTY 重开同一会话。HTTP provider 是确定性工具调用夹具，不是线上真实模型。本测试在隔离 Store/目录中用启动和完成文件观察物理执行次数；失去 Child 所有权后不按持久 PID 杀进程，等待有时限的原命令自然结束。

发现实际产品错误：`/recover block` 提交被 `completed tool Part lacks its original transcript call/result sources` 拒绝。原因是 TurnCompleted 的展示收尾把无原始结果回执的运行中工具标为 Completed。修复在 Core 增加工具展示 Sealed 状态；结束 Turn 时按同 Turn、同 call 的 pending Host effect 判断未知结果，封存展示并明确说明结果未知。公共投影复用已有 ActivityState::Sealed，trace 使用 Incomplete；不生成工具结果，不放松 tool source 校验，不把未知 attempt 改为成功或取消。

失败证据 `nola-external-crash-sdx3fzkc` 保留。更早的 `nola-external-crash-nsmv7969` 是测试在启动恢复尚未返回 composer 时过早发送 `/recover`；已改为等待实际恢复返回，不将“Type a message”首次出现当作空闲。修复后第一轮实际进程/PTY 验证通过，证据 `nola-external-crash-6ezfpft0`、`/tmp/nola-external-crash-fixed.log`；原 Run 唯一、物理命令只执行一次、provider 只请求一次、blocked seal 与 result 一致，原 attempt 仍为 outcome_unknown。

最终构建与严格复验通过：`/tmp/nola-external-crash-build-final.log`、`/tmp/nola-external-crash-final.log`，证据根 `nola-external-crash-2n_3_m4o`。Run `70134b1d-3b34-4b98-8d67-0afdc50ee8ca`，唯一未知 attempt `c491f661-a5a0-4aad-939d-3ce0ea352fac` 在封存前后保持同一身份；实际工具投影为 sealed，明确未知结果；原 shell 自然完成且启动/完成标记各一次，没有新模型请求。二进制 SHA256 `0edbeaf6ed2d573740dd0117981c2afbc92ba933b719bc8c2deca22206fda0de`。已有投影回归 10/10 通过：`/tmp/nola-external-projection-suite.log`；首次测试筛选后缀误用 `::tests` 命中 0 项，未计为通过，已使用实际模块前缀执行这 10 项。

这是“未知副作用不自动重放、用户明确保留未知并封存”的验证，不是崩溃后自动终止所有子进程的承诺；远程 Host 网络故障、执行中跨会话排队输入及长历史锚点仍待独立验收。


### 拉取最新代码及 CLI / Web 真实操作验收（2026-10-23）

保留工作区改动后合并 origin/Nola（上游 67057c8b，合并提交 7558592e），成功恢复暂存的改动，备份 stash 保留。未推送本轮修复。

本轮修复三条执行路径：

- 执行中 /resume、/continue 作为本地控制命令处理；返回会话后显式推进已持久排队输入，沿用原冻结模型/工具配置。跨会话 composer 与上下文不串入。
- 回执在转发前/完成后丢失，重复请求及 Host 重启只对账原请求，不重复创建 Run 或模型请求。模型连续两次输出截断后明确封存 incomplete，不再误判 answered；Store 只为相应 incomplete 结果接纳 model_output_limited 来源，保留其他来源校验。
- 320 条历史中恢复退出前阅读锚点和中文草稿。默认仍读最近 96 条；锚点超出尾部时使用已有完整 canonical history 读取，不是新增索引分页。/history 替换 archive 避免尾部重复；首帧前注册 SIGWINCH，重开后立即缩窗无需按键也重绘。

新增 scripts/tests/test_cli_remaining_acceptance.py，使用实际 CLI、service、HTTP、PTY/VT 与隔离 Store；故障 provider 为受控 HTTP，不冒充真实模型。最终四项通过（42.659s，/tmp/nola-final-remaining.log），证据目录分别为 nola-remaining-apfnqeaz、nola-remaining-lne7zhes、nola-remaining-kje13j8i、nola-remaining-f_t1w9lq。此轮 CLI SHA256 cea2a4e59e1e3b4c30dfcd8927245a938b2cb46065fcea2f43bcdaf992d2b278，service b0a2c29d61fd30e66e5b5ddea8bdfc768a41edac1ead7ae82dc2df4a11ab65b6。harness 校验运行前后二进制一致、唯一 Run/result/seal、物理请求数及清理失败。早期失败 /tmp/nola-remaining-final.log 保留；初次 Store 修复不足以解决第二次截断，随后修正 kernel 判定；历史测试亦实际发现首帧前未注册 resize 信号，未以重跑掩盖。

真实 Chrome 通过原生点击、输入、发送、刷新和切会话操作隔离 Web（端口 57717），使用现有配置声明的真实 provider，未修改生产凭据/数据库。会话 A 记忆代号、80 条长输出期间切换新建 B、B 不知道 A 代号、A2UI 表单生成及实际填写“验收用户-427”并点击确认，五个 Run 全部 answered。表单 revision 2 和输入值在 Host 重启后保留。专家定义通过正式 Host 命令准备，选择与发送通过页面真实点击，不能声称通过 UI 创建专家。

真实操作发现并修复：浏览器刷新丢失内存中的已提交 ticket，却需继续对账尚未确认的 source receipt；改为按完整冻结身份从当前持久仓库精确恢复，保留后续草稿且拒绝身份不匹配。另一个是新上游 Room 关联错误要求 Conversation ID 为 UUID，导致 Web conv-* / CLI 命名会话专家面板 409；改为接受当前 opaque 会话 ID，仍验证会话存在，Room 自身 UUID 规则不变。

验证：Web 487 项单测、Svelte 0 错误/警告；kernel completion 14 项、Store source 1 项、Room contexts 13 项、CLI interaction 28 项、terminal UI 74 项通过。构建成功；现有大 chunk / linker eh_frame 警告未在本轮解决。真实 UI 证据根为 /private/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-live-ui-mxsuo5i3，ui-acceptance.json 区分初始与后续构建，五个对话 Run 并非全部运行于后续专家 ID 修复二进制。

本轮未重验原生 iOS/macOS/Android；远程故障验收覆盖 HTTP Host，不等于移动端加密桥全覆盖。首次浏览器加载较慢，刷新后恢复，根因尚未确定。双专家真实执行首次一位 answered=42、另一位 execution_unavailable，保留失败记录，继续定位，不计为通过。


收口记录：双专家首次真实点击一位 completed、一位 execution_unavailable（无准入 Run，source receipt 为 rejected/dispatch_not_admitted）。后续在原上下文和全新 Room 分别通过正式 HTTP 入口再发两位专家，四个请求全部 completed；这些是接口诊断，不能算真实点击复验，也不能消除首次失败。已补 Room 准入拒绝日志，仅记录 HTTP 状态和受限机器错误码，不输出 provider body/凭据；首次失败没有细分错误码，根因尚未确认，未伪造恢复或改写失败状态。

最终共 10 个实际模型 Run 全部有一致的 result/seal，另有上述 1 个准入前失败的专家提交。ui-acceptance.json 的 passed_all=false，分别标记 Chrome 与 HTTP 入口。Chrome 窗口随后不可用（cgWindowNotFound），重新绑定仍失败，已请求用户重新打开；最终诊断构建的点击复验未完成。测试 Host 已在全部 Run 封存后正常停止，私有配置副本与测试凭据已清理，保留隔离数据库和证据。

诊断构建 CLI SHA256 05e0884272a65935fcfad9f03b79f4080ce54b1348d0faed9377f06b7ad5adaa；service 未变。四项实际 CLI/HTTP/PTY 验收在该构建再次全部通过（24.408s，/tmp/nola-final-diagnostic-remaining.log），证据 nola-remaining-93kqeelx、nola-remaining-3q6hn1wk、nola-remaining-dkb_3nh5、nola-remaining-7awr08iu。git diff --check 通过。当前不是“全面测试全通过”：多专家偶发准入失败、首次页面加载延迟与本轮原生平台验收仍未关闭。


### iOS / Android 模拟器真实页面任务（2026-10-23）

用户要求在模拟器页面真实完成多个任务，并明确同意在无可见窗口时使用 XCTest（iOS）和 UIAutomator/ADB（Android）进行点击、输入和截图。本轮使用当前工作区重新构建，未使用旧安装包冒充当前版本；没有 mock 模型回复、伪造执行事件或直接写消息数据库。iOS 的 -ui-testing 仅用于隔离运行数据，Android 在原先未安装 Nola 的 emulator-5554 上从首次引导开始操作。未清空既有应用数据，未接触真机。

证据根：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-mobile-click-al9xp5wi`。真实 provider 使用 ~/.nola/config.json 当前 default_model 所指连接；透明本机 relay 注入进程环境凭据，客户端页面填写本机地址，不向测试日志/截图写入 API Key。relay 原样转发请求与流式字节，不生成模型回复。请求、传输事件、截图、控件树与只读 Store 副本分别保存。Android 模拟器时间与主机时间不同，关联以请求/Run 身份为准，不按屏幕时钟拼接。

**iOS：两项完整场景通过（399.173 秒），没有跳过。** 使用 Nola-Mobile-UI-QA-20261023（A08FB5BF-E10E-4ACB-8922-BC0F1FA906F9，iOS 27）。执行 IOSRealConversationLifecycleTests 的 testRealA2UIFormEditSubmitRestoreAndIsolation 与 testRealProviderConversationIsolationAndLongRunNavigation：

- 点击模型中心，填写连接、发现模型、启用并在聊天页选择。
- 真实模型生成 A2UI 表单，点击一次审批，输入运行时随机名字，点击提交；同一 Surface 更新 ACK，输入框和按钮没有重复；关闭应用重开后表单和 ACK 保留；新会话返回 ISOLATED_NONE。
- 两轮记忆验证 FIRST_ACK / SECOND_ACK ORCHID731；12 段长任务期间边缘返回列表并重新进入，最后出现 NATURAL_TASK_DONE。
- 第二条长任务确认已转发到上游后离开再返回，点击停止；随后 POST_CANCEL_ACK 正常完成，新会话不继承代号，返回旧会话滚动找到早先回复。

只读 canonical Run/result/seal 核验：两个隔离 Store 共 9 个 Run，8 answered、1 cancelled，全部 sealed，身份与结果一致。iOS app 可执行文件 SHA256 `e93680fabac5e3a2eee211582355a403864ad483705543d0c7b3cb9535bcadcb`。ios-report.json、ios-real-click.log、ios-real-click.xcresult 记录完整证据。这不代表真机键盘、所有模型及多专家均已验证。

**Android：多个页面任务完成，但整体未通过。** emulator-5554（nola_a2ui_isolated_20261021），当前 APK 经过 Rust/Kotlin 原生构建后新安装；APK SHA256 记录在 android-final/report.json。通过实际页面完成首次引导、API 配置保存、点击 Use 选中模型、新建聊天、发送与返回：

- 第一轮 17×19 得到 323；第二轮没有重给代号，正确回忆 CEDAR731 并得到 323+19=342。
- 另建会话得到 ISOLATED_NONE，未串入代号。
- 真实模型生成表单，滚动到审批按钮并仅允许一次；填写 APRICOT852，实际点击 Submit acceptance，收到 ACK APRICOT852；表单 revision 2，冷启动后值和回复均保留，另一会话历史也正确。
- 40 条天文事实任务执行中返回列表，出现 Running，重新进入仍在执行。但流在第 20 条中途结束，canonical Run 为 failed，错误为 Model stream ended before an explicit completion event；当前页面仅显示截断正文，没有明确显示失败原因。relay 记录上游正常关闭读取，没有该请求的 client_disconnected。不能将此任务记为完整成功。通过页面明确请求续写 20–40 后收到 LONG_TASK_DONE，该独立 Run 为 answered。
- 点击停止另一个已转发上游、HTTP 200 的长任务后，出现 Command outcome pending: active-submission:a49b074d-b617-4689-a269-b0477b8b7093。界面恢复可输入/发送，但 Run ca2f7073-4098-456d-b9b8-2d1112738425 仍为 running，无结果与 seal；返回列表也不再显示 Running。冷启动重新进入原会话显示空态和 No model configured，pending 错误仍在。未发送新消息覆盖该异常，也未改写数据库。relay 已确认原连接断开，客户端停止不等于 durable cancellation 完成。保留现场后停止测试应用，待修复 AndroidApplicationRuntime / ApplicationSubmissionJournal 对账及 UI 恢复链路。

Android 共 8 个 Run：6 answered、1 failed、1 running 未封存。已封存结果与 seal 一致；没有把最后一项伪装为 cancelled。另记录体验问题：任务已结束后权限状态仍显示 Saved; waiting for the running task；render_ui 审批直接铺开大段 JSON，操作需多次滚动；英文界面的 A2UI 版本按钮仍为中文。它们没有在本轮修复。

工具证据限制：首次启动与一次流式输出中 UIAutomator 无法取得新树；android-53 的 XML 为旧树，已明确标为 invalid 并排除，临时抓取器随后改为验证 dump 成功再读取，后续截图/树来自新的成功抓取。截图只证明相应帧，不能据此宣称动画全程流畅。用户要求的真实操作已经执行，当前结论为 iOS 所测流程通过、Android 存在可复现的停止/恢复失败，不能合并为“移动端全部通过”。


收尾补充：两项 iOS XCTest 的用例与套件明确输出 passed，Store 在停止构建进程前已经全部 sealed；但 xcodebuild 在应用/测试进程退出后仍卡在诊断收集超过三分钟，SIGINT 未能完成，最终仅终止本轮 xcodebuild（退出 -15）。xcresult 缺少 Info.plist，附件导出失败，不能宣称整个 xcodebuild 命令成功或结果包完整。已保留实时模拟器截图、完整用例操作日志、relay 记录和 canonical Store 副本；ios-report.json 明确 result_bundle_complete=false。真实执行结论与测试工具收尾失败分别记账，未重跑覆盖。精简截图和报告另存 `.artifacts/mobile-real-ui-20261023/`。测试 relay 已在全部网络请求结束后停止，Android 测试 app 已停止；既有模拟器和失败数据库保留，没有清除数据。Android 取消后的续发验证因原 Run 未封存而未做，未隐瞒为成功。

### 移动端问题修复进行中（2026-10-23）

- Android 停止命令遗漏当前协议必填的 `cancellation_id`，四个入口统一生成该身份；不是模型拒绝停止。原失败请求没有 Core receipt，保留原始 journal 和 Store，不转换旧请求。
- Android 读取历史的 journal snapshot pin 与会话写入共用排队范围，导致同会话停止待确认时连历史都无法显示。快照 pin 使用独立排队范围，业务写入仍被同会话未确认命令阻挡。
- Android 失败消息显示持久失败提示；未确认写入进入 AwaitingRecovery；终态后刷新权限归属；A2UI 历史版本入口改用本地化资源。
- iOS 真机截图中的 `invalid room identity or title`：新会话允许空标题，但 ensure_for_conversation 将其原样交给非空 room title 校验。创建新 room 时为空标题提供名称，不修改原会话数据。Core agent_room_contexts 14 项通过。
- 普通 iOS 对话输入框补齐 `@` 专家候选和已选专家标签，沿用现有 IOSRoomDraft 语法及 expertSelection 提交链。新增真实模型 UI 用例覆盖空白会话选择专家后首条发送。
- 编译及修复后的真实页面回归尚在进行；不能将代码已修改记成全项验收通过。回归证据暂存 `/tmp/nola-mobile-fix-retest`。

#### 修复后真实验证

- iOS 最终 build-for-testing 成功；真实专家 UI 用例执行 1 项、通过 1 项（37.779 秒），不是仅构建成功。输入 `@Arith` 只显示匹配专家，点击后选中；空白会话首条实际请求返回 `EXPERT_RESULT=323`，对应新 Run 的 result/seal 核对为 answered。
- 移除 iOS ChatComposer 原生 inputAccessoryView 工具栏（绿色 ✅ 及其 44pt 高度）；导出的真实键盘截图确认无此按钮。保留聊天滚动交互收键盘，并更新相关 UI 测试。
- Android 18 项相关单测通过，独立数据库真实 JNI 当前取消协议测试 1 项通过。
- Android emulator-5554 原故障 Store 保留。覆盖安装后真实点击原会话，历史正文与模型恢复显示；未转换缺少 cancellation_id 的原始请求，也未删除该 Run。
- Android emulator-5554 独立用户 10（NolaFixAcceptance）：实际配置模型 → 长任务请求已到上游（HTTP 200）→ 点击停止 → 下一条计算回复 `RECOVERY_RESULT=161` → 强制退出、重启、重新打开同一会话。待确认 journal 为空；数据库核对两个 Run 分别 cancelled、answered，均有唯一匹配终态和 seal；权限恢复“Permissions saved”。
- 第一轮 iOS test-without-building 因测试二进制未包含新增方法而执行 0 项，不计入通过；重新构建后执行上述真实用例。
- 证据摘要与截图：`.artifacts/mobile-fix-20261023/`。完整日志、xcresult、隔离数据库位于 `/tmp/nola-mobile-fix-retest/`。
- 尚未完成：原始错误格式 pending 请求的明确拒绝/人工恢复体验；超长权限参数展示；流式失败提示的新故障端到端回归。不得将这些计为已验收。
- Android 最终覆盖安装真实复验：取消消息显示 `Cancelled`，不会再把 Core Cancelled 状态画成 Model request failed；后续真实回复和权限已保存状态在冷启动后同时可见。原故障会话可读，并明确显示待确认请求、禁止新输入，不再假装正在生成。


### 普通对话跨端专家补全与真实回归（2026-10-23）

本轮将 `@` 候选接入 iOS、macOS、Android、Web 的普通对话输入框，复用当前 Core 专家目录与提交链，选中项保存稳定身份，不通过名称猜测工具或路由。Apple 共享候选面板和 Unicode 安全查询解析；各端使用输入框附近的整行候选、姓名/说明、已选标签与移除操作，桌面方向键、Enter/Tab 选择、Escape 关闭；输入法组合期间不抢提交。当前查询范围为文本末尾，并非任意中间光标位置的完整 Telegram 编辑语义。

真实操作发现并修复了组件单测未覆盖的问题：

- Android 显式收件人 admission 返回 selected，原执行入口仅接收 accepted，导致停在 Processing；当前同时接收可执行阶段。已冻结专家消息不再校验不存在的私有编辑草稿；权限观察跟随 beginExecution 替换后的真实 session。
- Web 原生模型配置保留编辑器采样值，但能力未启用采样参数；Web 错将默认编辑值当作显式请求并拒绝。默认沿用能力开关，用户显式覆盖仍严格校验。相关 Rust model profile 42 项通过。

已完成的实际验收（无模拟模型回复）：

- iOS：原生 XCTest 点击普通空白会话、输入 @Arith、选专家、发送，真实回复 EXPERT_RESULT=323。1 项通过，51.408 秒，完整 xcresult；原生输入/组合输入测试 4 项通过。最终本地化构建成功。
- Android：隔离 emulator-5554 用户 10，Compose 点击实际 MainActivity 的新会话、模型、候选、发送；真实回复 ANDROID_REAL=697，再新建无选中项串入。最终 APK 安装复验 1 项通过，6.55 秒；另实际冷启动打开历史仍显示专家姓名和回复。输入/提交相关 JVM 测试 9 项通过。原用户 0 的故障数据库未改写。
- Web：可见 Google Chrome 打开实际 Rust Web Host；筛选、点击、焦点保留、Enter 选第二专家而不发送、移除、真实回复 WEB_EXPERT_RESULT=377、刷新恢复、新会话选择隔离均通过。可复执行脚本 scripts/test_web_expert_mentions.py。Svelte 检查 0 错误/警告，Web 单测 487 项通过，生产构建成功。
- macOS：完整 NSWindow 中真实输入、点击候选、点击发送得到 MAC_EXPERT_RESULT=703，真实用例 42.212 秒，连同输入/恢复测试共 8 项通过，xcodebuild 成功。真实操作暴露原生编辑器缺少 SwiftUI focused 绑定，导致草稿已更新而候选面板不出现；补齐绑定后通过。此前隔离凭据初始化与候选失败不计通过。测试使用 Test 产品目录和独立支持目录，未替换用户的签名 Debug 应用。

证据摘要和截图位于 .artifacts/expert-mentions-20261023，详细临时测试日志保留。限制：未验证真机第三方键盘；原 Android 错误格式 pending 人工恢复体验、超长权限详情等前述未完项仍未关闭，不能将本轮结果泛化为全平台全部问题已解决。

### iOS 真机截图问题复查（2026-10-24）

- 专家时间线重新按 wall clock 排序普通消息，时间缓存缺项以 0 排序，破坏 canonical 顺序。保留普通消息原顺序，专家回复通过已确认 submission 的 parent_message_id 关联到原问题，禁止靠私有 Context 时间戳猜顺序。
- 专家发送完成 admission 后原先只刷新会话设置，没有刷新主会话消息；问题暂时不显示，重进后才出现。补上已确认消息加载，并对异步返回校验当前会话归属。
- 专家 Workspace 漏订阅 committedAssistantJournalContent，只有消息元数据更新，正文/思考增量直到终态才显示。接入普通聊天已有 ChatMessageDraftReducer，独立跟踪 journal 游标并拒绝旧序列/异源更新；滚动跟随同时观察 journal 序列。
- 活跃回复的 reasoning 不再因为出现正文就移入折叠的历史过程。仅展示 provider 实际返回的 reasoning，不从正文生成思考。
- 专家时间线的最新按钮不再在任意 4pt 拖动后出现；离底部超过 180pt 才显示 44pt 圆形入口。去掉专家整块彩色背景、重复底部运行区；停止/审批/恢复位于对应回复处，首个回复尚未出现时仍保留控制。头像采用图标，避免短姓名在头像和姓名栏重复。
- 首轮真实模型两轮任务及滚动用例通过，但截图暴露用户问题缺失、流式中只显示 Processing；因此不作为完整验收。新增真实页面断言：问题在回复之前立即可见，首段正文须在最终标记前可见。最终复验结果待补充。

最终复验：当前代码 build-for-testing 成功；12 项定向测试全部通过（含 journal 独立于消息元数据增量、旧序列及异源隔离、排序和思考保留）；真实模型 UI 用例 1 项通过，75.792 秒。页面实际完成专家选择、首条问题立即可见且位于回复之前、第二轮 24 条长输出、首段先于最终标记可见、滚动看历史、点击最新按钮返回底部。增量截图约在用例第 52 秒取得，最终标记约第 67 秒出现。完整 xcresult：/tmp/nola-mobile-fix-retest/ios-timeline-stream-final-20261024.xcresult；截图及控件树：.artifacts/ios-timeline-20261024/。

测试修正记录：用户问题恢复可见后，旧的结束标记 CONTAINS 查询误匹配了问题文本，已改为精确匹配回复段落；失败记录保留，未把它当成功。provider 本轮只提供正文，真实 reasoning 增量未单独验收；reasoning 走同一 journal 订阅并有独立增量/展示回归覆盖，不能声称已在真机验证。未修改真机数据、未提交或推送本轮代码。

### 移动端信息层级减法（2026-10-24）

对照 docs/design/nola-ui-prototype 的 390px 实际浏览器渲染与现有客户端，主要差距是常驻操作密度，而非缺少装饰：iPhone 原生导航和自定义工具栏叠加，顶部专家/进度/资料/设置并列，输入框下仍持续展示正常权限状态。原型的 32px 图标与小字号不直接照搬到原生端，保留 iOS 44pt / Android 48dp 操作目标。

本轮落地：iPhone 使用一层原生导航（返回、模型、更多），复用原模型菜单；专家、进度、资料、会话设置放入更多菜单。Android 将四个常驻入口合并为更多菜单，模型区域获得额外宽度；Host 专用执行器入口仅在适用模型存在时呈现。两端输入区仅在 saving/pending/failed 时显示权限提示；正常状态与后台 checking 不再常驻占行，权限策略仍在原设置页编辑，实际审批卡继续显示。

适用范围是移动聊天主界面；本轮没有宣称模型中心、数据中心及所有桌面/Web 页面已重新设计。当前专家收件人标签保留，用于确认下一条消息发给谁，不与运行状态混为一谈。后续页面继续按主操作、按需展开、需要处理的异常三个层次审查。

验证结果：iOS 最终 build-for-testing 成功；12 项定向单测及真实专家会话 UI 用例通过（UI 105.565 秒），涵盖更多菜单、专家入口、正常权限栏隐藏、真实两轮回复与长输出滚动。该轮 xcodebuild 在测试结束后卡住结果收集，已终止测试进程，因此不计完整 xcresult 成功；保留测试日志与实际截图。Android 最终 APK/测试 APK 构建成功；在隔离 emulator user 10 真实点击更多菜单、专家面板完成、@ 补全、发送得到 ANDROID_REAL=697、新会话无专家残留，用例通过（13.403 秒）；随后手动点击历史会话确认实际回复与简化布局。证据：.artifacts/mobile-simplify-20261024/。

失败记录保留：Android 首轮发现 checking 权限状态常驻，已修正；第二轮因模拟器锁屏没有 Compose 层，不计产品通过；第三轮使用返回键后专家面板仍在，补全查询匹配两项，最终用页面“完成”按钮正常关闭后通过。系统返回键/输入法关闭顺序尚未独立验收，不能把完成按钮路径等同于返回键通过。未测试真机第三方输入法；未修改用户主数据或生产凭据。


## 2026-10-24：工作区合并与客户端本地指令落地

本轮修改正式客户端，不是原型：

- iOS / Android 主导航缩为「对话、工作区、设置」。工作区承载资料、记忆、计划、记录；模型管理保留在设置及对话模型入口。macOS 将计划放入现有数据工作区；Web 移除独立自动化导航，将原有真实计划管理页放入工作区分区。继续使用既有服务和 Store，没有建立第二套业务状态系统。
- 四个图形客户端提供显式 `/` 指令补全；Apple 两端共用 `Shared/Chat/ComposerCommand.swift` 和建议面板。`@` 专家选择保持原有行为。匹配只针对输入起始处的显式 `/`，不按自然语言关键词路由。
- 支持模型、专家、资源、权限、计划、搜索、停止、帮助。点击候选只补全，执行与补全分开；未知指令或多余参数显示错误并保留输入；指令在消息提交/专家分发之前处理，不进入 Agent 队列。
- 执行期间允许编辑本地指令；附件导入与普通消息仍使用既有权限和提交约束。停止使用原有停止操作，不伪造终止回执。空闲 `/stop` 明确反馈。
- `/search` 在 Apple 手机/Android 打开会话列表并传入搜索词；macOS/Web 使用现有会话内搜索。原生 `/plan` 打开现有计划管理页，**不支持参数创建计划**，带参数明确拒绝并保留草稿。Web `/plan 标题` 可打开现有编辑表单预填名称，仍须用户填写实际执行配置并保存。没有把 UI 指令偷偷转换成模型请求。
- Web 选中分区改用边框过渡，移除重复工作区标题及概览统计块。补齐导航变更的既有 UI 测试调用方。

验证记录（隔离数据，不改写用户会话或凭据）：

- Web `npm run check`：0 errors / 0 warnings；生产构建通过；`composerCommands.test.ts` 通过。
- Chrome + 实际 Rust Host（5141）真实点击通过：390、1440、320 宽度；新建会话、候选点击、无效命令、搜索参数、空闲停止、帮助、计划及四分区切换。网络观察确认这些指令 **0 次消息提交**。脚本 `web/playwright_runtime/workspace_commands.cjs`，截图 `.artifacts/workspace-commands-20261024/`。未将这些 UI 测试声称为真实模型推理测试。
- iOS 模拟器真实 XCTest 点击通过：3 个主导航、4 个工作区分区、补全不执行、错误保留草稿、新建另一会话、搜索返回与搜索词保持。结果 `/tmp/nola-workspace-command-ios-acceptance.xcresult`；共享解析单测同次通过。期间发现并修正父级 accessibilityIdentifier 覆盖分区标识和搜索返回时搜索框隐藏问题。
- Android 隔离 emulator user 10 正式 Activity 点击测试通过，`WorkspaceCommandProductUiTest` 1 test / 0 failures（24.644s）；包含工作区各分区、补全、无效指令、帮助与 `/plan` 导航；日志 `/tmp/nola-workspace-android-final-ui.log`。构建通过，测试后切回 user 0。
- macOS `Test` 配置编译和 `ComposerCommandTests` 通过，日志 `/tmp/nola-workspace-mac-final.log`；未覆盖本轮 macOS 实际窗口鼠标验收，不把单测当作 UI 点击测试。

补充回归：`CoreUserFlowTests/testPrimaryTabsOpenTheirOwnScreens`（32.618s）和 `testModelCenterExposesEveryConnectionPathWithoutHorizontalPaging`（19.324s）实际 iOS 点击通过，结果 `/tmp/nola-workspace-navigation-ios.xcresult`。Android 更新后的旧导航测试调用方编译通过。Web 最终清理后再次检查为 0 errors / 0 warnings。

### 2026-10-24 真机停止未确认后的重复提交

- 日志显示先出现 `cancellation_control_unconfirmed`，随后客户端允许再次发送，Core 正确拒绝 `assistant turn cannot overlap an active Run`。客户端普通 error 状态不能作为 durable Run 已结束的证据。
- iOS 在清空草稿/暂存消息之前，以及 ChatManager 执行入口再次读取 Core 当前 activeRun；未结束时保留草稿并进入既有恢复检查。停止操作也必须验证 Core 已结束才显示停止完成，不改写或删除用户 Store，不伪造停止证明。
- 紧凑工具栏改为挂载 ChatTopBar View 的不同展示模式，不再直接读取未安装 View 的子属性，修复 DynamicTypeSize / Bool Environment 读取路径。Room 几何状态忽略亚像素变化，并推迟到下一轮主线程发布；仍须以设备日志确认其他布局循环。
- 7 项取消生命周期测试及工作区实际点击测试通过，结果 `/tmp/nola-ios-cancellation-admission-retest.xcresult`。含未确认物理请求不能产生 terminal seal，且 activeRun 阻止下一轮的回归。
- 真实模型 XCTest 实际导入 `qwen3.8-flash-next-iq3_s`，新会话发起长回复，透明转发器确认请求已到真实 upstream 后点击停止，立即发送后续消息并收到 `CANCEL_FOLLOWUP_OK`。本次走停止确认分支，60.910s / 0 failures，截图与可访问性树保存在 `/tmp/nola-ios-real-cancellation-run.xcresult`。未确认停止分支由上述生命周期测试覆盖，不能声称本次真实模型复现了未确认分支。首次真实测试因环境变量未传入 XCTest 被跳过；修正 TEST_RUNNER 环境传递后以上重跑实际执行成功。

### 2026-10-24 iOS 模拟器真实模型探索回归

使用隔离 iOS 27 模拟器和临时 Store，从 `~/.nola/config.json` 读取默认模型连接，通过仅本机透明转发器注入凭据、转发实际 upstream SSE；不伪造模型回复、不改写用户数据库。通过 XCTest 操作正式界面的模型导入、输入、按钮、侧滑与重启，并检查截图。桌面 CUA 原生管道本轮不可用，未把静态检查冒充人工窗口点击。

真实操作发现并修复：

- 长 A2UI 内容把当前权限卡片留在上方，自动跟随停在内容底部。初次长更新测试在此停滞并主动中止，未计作通过。将当前权限卡片移至本轮生成内容之后，与跟随位置一致，仍使用原请求及审批回调，未放宽授权。修后五次独立更新、页面往返和随后长更新取消完整通过（363.329s）。
- A2UI primary 按钮没有使用现有 `NolaOnAccent` 配对色。共享 Apple Renderer 现在明确使用 AccentColor / NolaOnAccent；资源色值计算深色文本对比度从 2.08:1 提高到 8.15:1，浅色为 6.39:1。macOS Test 配置构建通过。
- 表单历史版本入口硬编码中文，改为 iOS/macOS 本地化文案。
- 已运行会话列表仍显示“还没有消息”，刷新时失效预览缓存，运行中没有摘要时只显示实际运行状态；补对应 UI 断言。

修复后验收（结果 `/tmp/nola-ios-exploratory-fixed.xcresult`）：

- 中文多行草稿、键盘位置、专家面板、会话切换、冷启动恢复后真实发送：75.088s 通过。
- 真实 A2UI 表单编辑、提交随机值、更新原 Surface、重启恢复、新会话隔离：139.721s 通过。
- 真实 A2UI 长更新、页面往返、取消及最后已提交内容保留：363.329s 通过。
- 普通真实多轮上下文、自然完成长任务离开/返回、另一长任务停止及续聊、新会话隔离、上翻恢复早期记录：212.109s 通过；以上 4 项合计 790.248s / 0 failures。
- 专家 `@Arith` 补全、未命名会话首条实际回复、长回复首个 delta 早于结束标记、上翻及最新消息返回：94.306s 通过。同期旧 fixture 键盘用例失败，整组未计为通过；列表筛选与草稿恢复独立通过。失败诊断收集挂起超过 10 分钟后终止了对应 xcodebuild，仅保留原始日志 `/tmp/nola-ios-exploratory-experts.log`。
- 像素验收增加：截图裁切模型选择和发送/停止控件区域，验证不仅 AX 存在，而且实际像素有可见内容。真实表单及停止续聊重跑 2 项 / 0 failures（150.991s、61.821s），结果 `/tmp/nola-ios-visible-controls.xcresult`。
- 更正截图观察：查看工具对部分带元数据的 iOS 截图显示出标题/输入区空白，但原始 RGB 像素中控件完整，去除元数据后显示正常。这不是已复现的 App 故障，不列为修复。已撤销仅基于该误判的顶栏宽度调整；保留像素断言与原始截图用于核实。
- 旧 viewport 用例按自绘顶栏断言 AX minY，与当前原生导航安全区不符；键盘/历史补充真实模型流程，同时保留原 Host fixture 回归，不删除业务断言。输入框 UIKit 委托只在焦点确实变化时回写 SwiftUI，并拒绝以零/非有限宽度测量文本。
- 截图证据：`.artifacts/ios-exploratory-20261024/`；macOS 共享 Renderer 编译：`/tmp/nola-apple-renderer-contrast-mac.log`。
- 真实键盘回归初跑也失败：长回复完成后拖动消息列表，键盘未收起，`/tmp/nola-ios-real-keyboard-navigation.xcresult`。这是正式路径可复现问题，不归为 fixture 假失败。消息列表改用原生开始拖动即收键盘，避免交互式收起与编辑器焦点同步争用；保留草稿和无额外键盘工具栏要求；相同真实流程复验结果见下条。
- 真实键盘修复重跑通过（140.928s），结果 `/tmp/nola-ios-real-keyboard-fixed.xcresult`：长回复后输入草稿、消息区拖动收键盘、跨标签返回、草稿和已提交回复保留，且模型/发送控件像素检查通过。

- 未通过项明确保留：旧远程 Host fixture 在返回会话后显示 `Waiting for the Host to confirm the original submission`，缺少已提交消息，原始可访问性树 `/tmp/nola-fixture-return-tree.txt`、结果 `/tmp/nola-ios-fixture-final.xcresult`。将拖动坐标限定在可见消息区域后键盘断言通过，历史恢复仍失败。尚未确定是 fixture 协议覆盖不足还是远程恢复正式路径问题，不能标记已修复，也不能用真实直连模型通过替代远程 Host 验收。

### 2026-10-25 跨端逐项真实操作复验（进行中，未宣称全按钮通过）

- 使用独立 iOS 模拟器数据、Android user 10、Mac Test 输出和临时 Web/CLI Store。真实模型来自用户配置，经本地透明转发；没有伪造模型输出。桌面 CUA 管道仍不可用：浏览器使用实际 Google Chrome + Playwright，移动端使用 XCTest/Compose 触摸，Mac 使用实际 NSWindow 控件事件。
- 已定位上一节远程 fixture 的问题：缺少当前 `GET /agent/settings` 契约，第一次发送实际失败，而旧断言把错误气泡当成 completed 回复。补充 fixture 并要求正文 `canonical Host stream` 在导航前后都存在。5 项 fixture 单测和该键盘/历史 UI 回归通过：`/tmp/nola-allplatform-fixture.xcresult`。
- Android 真实模型导入、滚动到“使用”按钮、切换 Chat、发送和显示长回复通过（62.926s）：`/tmp/nola-allplatform-android-nav.log`。旧测试点击屏幕外控件、按未选中底栏隐藏文字找入口均已修正；不将测试修正记为产品修复。A2UI 回看测试也需使用当前带会话身份的列表键，已修正，复验尚未结束。
- macOS 真实专家回复得到 `MAC_EXPERT_RESULT=703`：`/tmp/nola-allplatform-mac-real.log`。实际窗口内面板、按钮、输入、长内容边界 29 项通过：`/tmp/nola-allplatform-mac-controls.xcresult`。这是控件集覆盖，不是所有业务/真实模型任务覆盖。
- macOS A2UI 实际编辑、点击提交并出现随机值 ACK，但后续重复发布相同 Surface 直至超时，保留失败：`/tmp/nola-mac-real-memory.46TK1V`。不能仅以表单显示判定成功；当前没有宣布修复。
- Web 实际 Google Chrome 已发送并收到 `WEB_BUTTON_AUDIT_OK`，覆盖宽屏/手机面板、设置七页、工作区四页和新建能力弹窗。发现新建能力未编辑时仍提示放弃修改，以及未保存对象显示删除按钮；正在修复复验。逐控件记录在 `web/playwright_runtime/real_button_sweep.cjs`，失败报告 `/tmp/nola-web-sections-buttons/report.json`；移动面板偶发超时仍单独保留。
- iOS Host 配对成功，但系统 Password AutoFill 提示和转场会拦截后续导航，真实远程双专家任务尚未走完；不以直连模型通过替代。原始失败 `/tmp/nola-allplatform-ios-host-actions.xcresult`。令牌输入语义与转场等待正在复验，尚不能宣称提示彻底消除。
- CLI 通过真实 PTY 操作 `/help` 并收到 `CLI_REAL_BUTTONS_OK`，只读校验终结 seal/digest 为 answered。运行服务读回的验收连接失败，完整用例仍失败；证据 `/private/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-allplatform-4z4qew2j/`，不伪称完整通过。
- 尚需覆盖：远程取消/故障和双专家完成链；Android A2UI 提交恢复；Mac A2UI 重复发布；模型管理增删改与非破坏性设置动作；所有页面按钮完整清单。删除、导出、权限操作仅用独立测试对象，保留用户数据。

进展更新：

- Web 能力编辑修复后，Google Chrome 设置七页、工作区四页、技能/MCP 新建取消与修改后继续编辑/放弃，共 37 次点击通过，无 pageerror：`/tmp/nola-web-sections-fixed/report.json`。新建未保存对象不再显示删除按钮；新增对象仍可保存，既有对象只有实际变更才启用保存。
- Android 真实 A2UI 复验通过（85.956s），含原 Surface 更新、随机提交值、Activity recreate、新会话隔离及持久化 revision 对齐：`/tmp/nola-allplatform-android-a2ui-awake.log`、`/tmp/nola-android-a2ui-final-result.json`。另一次独立进程回看及再次重建也通过：`/tmp/nola-android-a2ui-cold-reopen2.log`。
- iOS Host 导航需等待系统模态层完全消失后控件实际可点击，不能仅以 AX existence 判定。加入真实点击前 isHittable 等待后，已经进入远程请求并生成两条真实子任务；最终完成断言尚在执行。
- Web 完整流程复跑通过，56 次实际点击、真实回复及返回原会话验证，无 pageerror：`/tmp/nola-web-buttons-final/report.json`；`npm run check` 0 errors/0 warnings，生产 build 通过。此前移动面板超时仍留存，单次通过不证明其已根除。
- iOS 双专家任务运行到子任务 `waiting_permission`，旧用例未处理真实审批。已给该场景增加可见审批按钮点击和证据截图，等待复验；未对 Core 权限做自动放行，也未将等待授权计为模型失败。
- Web 实际保存链进一步暴露新建技能缺少 `category_id`、描述未标必填。已为新技能生成稳定 UUID 和 `custom` 分类，并将描述/内容设为必填。Chrome 实际新建保存、重新打开、修改保存、刷新重开、删除确认完整通过：`/tmp/nola-web-capability-buttons/report.json`；原失败显示后端 `requires canonical field description`，保留 `/tmp/nola-web-capability-buttons-fixed.log`。
- Mac 二次重复发布复现增加了仅针对隔离验收的可选模型末条输入记录（`NOLA_MAC_REAL_CAPTURE_TEST_CONTEXT=1`，默认关闭）。发现共享 Core requirements 来源将已接纳 `UiAction` 标成 `non_message/not_task_text`，丢失用户提交值。修为保留原 admission/origin、user 角色及同一规范 envelope 的有界摘录；没有增加工具授权或伪造完成证据。10 项 requirement_source_tests 通过：`/tmp/nola-ui-action-source-tests.log`。是否解决模型重复发布尚待重建真实验收，不将推断记为事实。
- 撤回 iOS 令牌 textContentType 实验：nil/空类型/oneTimeCode 都未可靠消除系统保存提示，恢复原语义；只保留实际点击“以后”并等待 tab 可点击的测试处理，不把无效尝试作为产品修复。
- iOS 远程复验进一步确认：子任务在独立 Host Store 中为 `waiting_permission`，而循环查询正式对话的 `chat.permission.allow_once` 始终不存在。当前移动端任务 projection 只有任务/指导字段，Run observation 只收当前根 Run 的权限；这条委派任务权限展示链仍有缺口，不能通过自动批准或换无工具任务规避。原场景保留为失败，等待专门修复。
- iOS 真实双专家用例最终失败：`/tmp/nola-allplatform-ios-host-permission.xcresult`。增加真实审批操作仍找不到按钮；两条子任务实际存在，其中任务为等待权限。需要补齐远程委派权限的拥有者校验、展示、决定回传、冷恢复和跨会话隔离，不能只显示按钮或自动批准。此项为当前未解决阻塞。
- CLI 初次服务读回失败源于使用不同构建时间的旧 CLI/Oracle 产物，不能代表当前源码。两者一起从当前源码重建后，真实 PTY `/help`、模型回复、服务 terminal seal 校验、退出后按同一会话重开恢复全部通过：`/private/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-allplatform-70aau6no/`。本次真实终端没有通过跳过服务读回来变绿。
- Web 正在使用新编译 CLI Host 的独立 5142 端口重新验证；此前 5141 结果仅证明当前前端与当时已有 Host 产物的组合。新的状态 `/tmp/nola-a2ui-current-host-state.json`，测试 Store 独立，未转换或改写旧 Store。
- Core 最终补充超长 Unicode UI action 截断回归，10 项全部通过：`/tmp/nola-ui-action-source-final-tests.log`；Web 能力逻辑 8 项通过：`/tmp/nola-web-capability-unit.log`。
- 最新源码编译的 Web Host（5142）完整真实 Chrome 流程通过：56 次实际点击、真实模型回复、宽/窄屏面板、搜索刷新、设置七页、工作区四页、能力取消确认及返回原会话，0 pageerror：`/tmp/nola-web-current-buttons/report.json`。此结果替代旧 Host 产物的最终验收依据。
- 最新 Host 的技能新建保存、重开修改、刷新、删除确认也通过：`/tmp/nola-web-current-capability-buttons.log`，对应 `/tmp/nola-web-capability-buttons/report.json`。
- Mac 重建后的真实 A2UI 仍失败：`/tmp/nola-mac-real-memory.XLeGtb/result.xcresult`。请求记录已确认 `source_inputs` 包含 `ui_action` 的真实提交值，说明来源修复生效；模型仍重复发布相同 UI，最后超时并在测试清理时报 `InvalidTransition idle -> failed(deinit)`。来源缺失不是循环的完整根因，不能标记循环已修。没有添加自动停止/伪造完成/跳过权限来通过验收。

本轮未提交、未 push。测试会话、截图、终端帧和独立数据库保留；Android 已恢复原用户和测试前超时设置。仍未达到“每个按钮、所有状态全通过”，尤其上述远程委派权限和 Mac A2UI 循环必须继续修复。

### 远程子任务审批桥接（继续验收，未完成）

- Host Run observation 原先只暴露根 Run 的临时权限等待器，遗漏子任务的持久化 permission_requests。新增父任务审批适配，继续复用现有客户端权限卡片和 Host permission response 协议。
- 所有权从已提交的 AgentTaskAdmission 校验：父 conversation/Core session/Run/turn、子 conversation、task 与原始 Core permission 的参数/指令必须一致。父 conversation ID 不等于 Core session ID，不能直接比较或把展示身份改成执行授权身份。
- 决策回传重读原始请求及任务状态，调用既有 Application 权限命令；保留 nonce/到期/精确响应幂等校验。过期观察不自动拒绝任务，不增加自动批准。单次观察仍最多 16 个权限项。
- 真实 Application/Core/Store 权限等待测试扩展：父会话可观察，其他会话/轮次不可观察，修改参数不得批准，原请求允许一次后恢复执行，重复旧请求拒绝。该测试和旧面板不能批准同任务下一请求的回归各 1 项通过：`/tmp/nola-remote-permission-final-tests.log`。测试需独立 NOLA_HOME 和 RUST_MIN_STACK=33554432；初次受本机模型配置和测试线程栈影响的失败保留，不冒充产品通过。
- 最新桥接 Host 上真实 Chrome 56 次点击及真实回复再次通过：`/tmp/nola-web-delegation-final-buttons/report.json`。首次页面导航超时记录 `/tmp/nola-web-delegation-bridge-buttons/report.json` 保留，尚未证明偶发超时根除。
- iOS 本轮真实验收仍未通过：第一轮未包含修正后的 Core session 身份映射；第二轮实际 tap 在模拟器超时；重启后重用旧地址触发正确的主机身份变更拒绝。分别保留 bridge/final/reboot xcresult；改用新隔离 Host 5143 继续真实配对和双子任务验收。不得移除身份检查或清除用户配对以使测试通过。
- 5143 配对成功后原地返回仍失败（`/tmp/nola-ios-delegated-permission-5143.xcresult`，tab AX window 为非有限 frame）；另设配对冷启动场景继续后续任务，不将冷启动作为该导航故障的产品修复。测试清空已保存 URL/令牌再输入，并校验地址，防止追加旧配置。
- `/tmp/nola-ios-delegated-permission-clean-fields.xcresult` 已真实完成配对、冷启动、选 Host、发送、返回会话并创建两个实际子任务，但 300 秒最终结果断言失败。本次 Store 没有 pending permission，因此不能证明桥接的真实审批已通过。只读追踪显示一个子任务 `tool_search {query: "select:bash"}` 得到 `0 tool matches`，模型输入 token 为 45150；随后继续模型请求，另一子任务仍在模型请求中。保留这个工具发现/上下文/长任务未完成问题，不增加关键词路由、不自动批准或伪造完成。
- 最终 `cargo check -p nola_cli --lib` 通过：`/tmp/nola-remote-permission-final-check.log`；当前完整任务仍未完成。5143 独立 Host/Store 保留用于继续观测两个任务，不影响用户原有 Host。

### 委派能力契约与真实失败复核

- 复核真实 5143 Store：找不到 Bash 的子任务被父模型设置了 `requested_read_only=true`。这是实际目录收窄，不是搜索实现漏掉可用工具。现有 Store 明确限制每项目一个命令/写入型活动 Agent；不能为满足并发描述而把命令任务降为只读。
- 修订 `spawn_agent` schema：需要命令的任务应等待现有命令任务结束，不能用只读模式绕过调度限制。响应回传实际 permission_mode/capability_scope，子 Agent 系统上下文明确只读排除命令、无法满足时向父任务报告而非反复搜索。
- 删除无独立授权含义的 `permission_mode=allow`，当前只接受 `ask` / `read_only`，省略按 ask。未知/旧值明确拒绝，三个 PTY 调用方更新为 ask；不改变 Core 的具体调用审批政策。
- 新任务尚处于 queued 时，工具摘要改为“排队中”，不再提前显示“运行中”。
- 真实运行另暴露一项仍待解决：`Store conflict: task turn admission identity, lease, prior Run, or prior permission differs`。保留 5143 独立 Store。测试 Host 已显式停止，避免失败验收结束后继续消耗模型请求；用户 Host 未操作。
- 本轮契约修订编译通过 `/tmp/nola-subagent-contract-check.log`，CLI 构建通过 `/tmp/nola-subagent-contract-build.log`；真实目录/配置隔离回归 2 项通过 `/tmp/nola-child-catalog-tests.log`。尚未以这些单测推断真实模型已纠正委派策略，下一步必须继续真实复验。

### 子 Agent 续接准入冲突：已定位的共享契约矛盾

- 真实失败任务 `task-1dd8b4bb8a5e` 复用了已结束任务 `task-1252af741ca1` 的子会话 `agent-4cb5f8181e1e44e2`。原始证据仍位于 5143 隔离 Host Store（`/tmp/nola-a2ui-current-host-state.json` 指向目录）；不是偶发租约抢占的推断。
- CLI `spawn_agent.task_id` 明确允许继续已完成任务，`resumable_child_session` 返回旧 child，创建新任务且 metadata/TaskLineage 标记 resumed_from_task_id。
- `core/nola-store/src/runtime_task_run_binding.rs::validate_admission` 却要求该 child 没有任何旧 binding、仅一条 durable_task、没有任何旧 Run。这会确定性拒绝上述公开能力，即使原 Run 已有终结凭据。
- 现有 `completed_agent_task_can_resume_only_inside_its_owner_boundary` 只检查 helper 返回字符串，未实际执行 BeginTaskAssistantTurn；不能作为续接成功证据。
- 后续修复必须以已提交的前驱任务/Run 终结事实和新任务准入为依据，支持同一子会话的新 Run；保留旧 binding 不可变、禁止重新执行旧 Run、禁止跨父会话/项目续接和并发分叉。不能简单删除 SQL prior-run 条件，不能只相信可变 metadata，也不能为旧存储增加迁移/回退。
- 必需验收：通过 Application 实际完成第一任务并提交 terminal receipt；再创建带前驱身份的新任务、申请新 lease、执行第二轮；验证历史上下文可见、旧 binding 未变、不同 Run、外部副作用各执行一次。覆盖未终结前驱、外部会话、伪造前驱和两个竞争续接，错误必须在新 Run 发布前拒绝；再进行真实 UI 续接。

### 子 Agent 续接准入：共享实现与回归（2026-10-25）

- Core 导出准入事实时携带实际 Task lineage 的前驱 task ID；Store 只允许沿已封存、具有精确 Run 绑定和终结凭据的同属主子会话链续接。旧 Run/绑定/凭据不修改，新任务创建独立 Run。无前驱、错误前驱、未封存前驱、不同项目及竞争续接均拒绝，事务不发布第二 Run。
- 准入 DTO 当前协议为 v2；v1 明确拒绝，不增加历史迁移、双读或自动清库。用户数据库未改写。
- `RUST_MIN_STACK=33554432 cargo test -p nola_application --test task_run_binding_e2e -- --nocapture`：18/18 通过，日志 `/tmp/nola-task-continuation-suite.log`。新测试真正完成并封存两轮子 Run，第二轮实际 ModelStream 请求含第一轮结果，原绑定/凭据保持不变；另覆盖五类错误前驱/竞争情形。
- Domain 准入摘要测试通过；service-client 33 项通过，日志 `/tmp/nola-admission-domain.log`、`/tmp/nola-admission-client.log`。这些属于共享逻辑/IPC 回归，不代替真机或模拟器 UI 操作。
- 当前版本 CLI/mobile bindings 编译与新版 Host 实测继续进行。远程审批、多 Agent 真模型任务及全部平台按钮仍未完成验收，不标记已解决。

### 新版 Host 与配对交互复验（2026-10-25）

- CLI/mobile bindings 编译通过，新版 CLI 构建通过；隔离 Host 端口 5144 已启动，未替换用户服务。
- 首轮真实 Chrome 在登录后 `/chat` 导航 20 秒超时，页面仍为加载状态；失败证据 `/tmp/nola-web-continuation-buttons/` 保留。第二轮同一 Host、同样超时限制完成 56 次真实点击及真实模型回复、历史恢复，`/tmp/nola-web-continuation-network/report.json`。第二次通过不代表首次偶发加载问题已修复。点击脚本增加不含 query/token 的失败请求路径及 HTTP 状态诊断。
- iOS Host 令牌输入移除网站密码语义，连接开始前解除焦点；应用仍在验证 Host 后自行保存 Keychain。新增配对后不重启应用直接点击会话入口的真实断言，再验证冷启动。该 Swift 改动的模拟器编译/运行尚在进行，不宣称原地导航故障已解决。

### 消息渠道未配置状态修复（2026-10-25）

- 真实 Web 点击发现未配置服务令牌时 GET channels 反复返回 400，UI 却显示保存未确认且要求重填秘密值。改为先读取已保存 worker 配置；缺少服务地址/令牌时提示完成配置，不发渠道请求、不冒充已加载的空渠道列表。真正读取失败独立显示读取错误，保留认证错误提示。
- Svelte check 0 错误/0 警告，生产构建通过。真实 Chrome 设置、资料/计划、技能/MCP及重新载入等 37 次点击通过：`/tmp/nola-web-messaging-setup/report.json`。页面树验证配置提示可见、旧错误提示消失，网络记录没有 channels 失败请求。未验证真实第三方渠道凭据的发送/接收，不能据此宣称消息渠道全功能完成。
- iOS 远程多 Agent 实测同一 xcodebuild/cargo 进程继续编译共享 release Core，未因日志静默重启任务。

### 远程子任务审批实时遗漏：真实复现（2026-10-25）

- 新版 Host 5144 + iOS 模拟器真实配对/新建/发送/列表往返后，任务 `task-cd5f4b1c28a7` 进入 `waiting_permission`，数据库有 pending 请求，但 iOS 没有 `chat.permission.allow_once` 按钮。截图 `/tmp/nola-ios-remote-permission-live.png`。
- 同一已配对身份通过真实 Web 登录会话只读 POST `/api/chat/agent/v2/observe` 得到 HTTP 200、recovery=observe、permissions=2。故不能归咎模型慢或没有生成审批；当前 delegated bridge 能供观察接口读取，却尚未把子任务审批送入持续中的 SSE。后续需在 Host 实时通道复用同一精确 owner/challenge 规则，并验证每个 nonce 去重、过期重发与结束清理；不能用前端猜测授权。
- 当前 iOS XCTest 使用构建开始时快照，日志没有新增配对后原地导航断言；不得据此声称新 Swift 配对修复已验证。待这轮结束后需要增量重建重跑。

### 子任务审批实时推送实现（2026-10-25）

- `delegated_permissions.rs` 增加运行生命周期内的审批推送：500ms 有界等待，复用精确 owner/当前 Run/当前 turn 的观察与审批 registry；同 nonce 只发送一次，过期新 nonce 可重发。首轮须先通过 submission receipt fence；恢复路径已验证原 Run 后复用相同推送。
- `native_bridge.rs` 首轮和 `permission_resume.rs` 显式恢复都接入；RAII 通知退出并 join，先停止推送再发送结束标记，避免留下后台推送线程。实际审批提交仍走既有 nonce/owner/持久化请求重新校验。
- CLI check/build 通过，日志 `/tmp/nola-delegated-stream-check.log`、`/tmp/nola-delegated-stream-build.log`；审批恢复回归在运行。旧 Host 5144 已停止，新隔离 Host 5145 就绪，模拟器 `/tmp/nola-ios-live-permission.xcresult` 重建重跑中。真实审批按钮和结果尚待这轮证明。

### 配对原地返回与审批队列补充（2026-10-25）

- `/tmp/nola-ios-live-permission.log` 新轮真实 XCTest 已执行 `host-paired-live-navigation`：配对后先点击 tab.chat 并找到新建会话，之后才冷启动。该证据覆盖此前被重启掩盖的原地导航问题；不代表所有系统密码弹窗都不会出现。
- 检查 IOSHostPermissionSession.start 发现新请求会取消当前展示。实时推送因此改为保持当前子任务 nonce，处理完成后再推下一个，防止多个待审批卡相互覆盖。CLI check 通过 `/tmp/nola-delegated-stream-queue-check.log`，当前运行 Host 5145 尚是该队列补充前二进制，不能把本轮结果算作队列补充的运行验证。
- Host 5145 的模拟器实测正在执行，已创建真实子任务 `task-fcdee6c86652`，审批/最终结果尚待验证。

### iOS 审批解析缺口（2026-10-25）

- 新增实时推送后仍无卡片。检查 WebAgentProvider.parseSSEObject：permission_request 分支之前无条件要求 Core Trace，但子任务审批是独立 Host challenge，原观察接口也没有 Trace。不得伪造父任务 Trace。
- 新增 delegatedPermissionRequest 事件，明确识别 task_id 的审批传输，保持当前协议版本/eligibility 解析；ChatManager 首轮/恢复都走既有完整 host/client/conversation/run/nonce/expiry 绑定检查。AgentTrace 适配不制造父任务轨迹。
- 新增解析回归：无父 Trace 的子任务审批可解析，错误协议/缺失 eligibility 拒绝。待 XCTest 编译运行。Android DTO 与 Web permission handler 已检查，无同样的 Trace 必选条件；仍需真实各端审批验证。

### 新版解析编译与回归证据（2026-10-25）

- Host 5145 原轮超时失败，结果保留 `/tmp/nola-ios-live-permission.xcresult`；测试 Host 已停止。
- 新隔离 Host 5146 使用已包含审批逐个推送的 CLI 二进制。iOS 新客户端编译通过；`WebAgentProviderTests.testDelegatedPermissionTransportDoesNotRequireOrInventParentTrace` 在模拟器通过（`/tmp/nola-ios-delegated-parser.log`）。同一构建继续运行完整真实多 Agent UI 流程，不能提前声称审批已完成。

### 首个远程审批真实闭环通过（2026-10-25）

- Host 5146 + 新版 iOS 的真实 UI 运行在约 176 秒找到 `chat.permission.allow_once`，截图 `/tmp/nola-ios-parser-live.png`；XCTest 实际点击允许。
- Store 请求决定为 `allow_once`；后续 closure source=`core_settled`（内部状态 withdrawn 是已结束请求关闭，不代表撤销授权）。子任务 `task-0376075b2ba5` 已 sealed，不可变终结凭据含 `CHILD_ONE=323`。这是实际模型/真实命令与点击的闭环，不是 DTO 或模拟响应测试。
- 第二个子任务 `task-bb379cdd1bf6` 已启动，但到本轮 UI 约 350 秒仍未完成。既有 300 秒完成窗口即将失败，不能把第一个子任务通过当作完整多 Agent 验收。需保留时限失败并继续定位串行执行/模型调用耗时及完整汇总。

### 同一远程任务的冷恢复验证（2026-10-25）

- 完整双 Agent 300 秒窗口失败后，保留 Host 5146 的第二个真实任务，新增 UI 用例只启动原隔离数据库、点击原会话、处理审批并等待两个结果；不发新消息，不重建 Store。
- 首次补充用例因 UUID 大小写定位错误失败（UI 标识为 lowercased UUID），`/tmp/nola-ios-cold-existing.xcresult` 保留；已改正确选择器，`/tmp/nola-ios-cold-existing-retry.xcresult` 重跑。此失败属于测试代码，不能归为产品会话丢失。
- 原任务阶段耗时：首个子任务创建到封存约183秒；第二个创建到开始34秒、到绑定62秒、到审批95秒。需进一步定位模型/调度/持久化各段，不把放宽超时当作性能修复。

- 冷恢复补充验证第二次失败仍为空列表：导出 AX 确认后发现 XCTest 安装重定位 app container，传入构建前的绝对 DB 路径打开了空测试库。原会话仍在被搬到新容器的相对测试目录中。改用原 `NOLA_UI_TEST_RUN_ID` 命名空间恢复，不复制/迁移/删库；`/tmp/nola-ios-cold-relative.xcresult` 在运行。两次失败证据保留，均不能声称产品恢复成功。

### 等待误判无进展：新的真实根因（2026-10-25）

- 相对测试目录冷启动已恢复原会话，但显示空闲、未恢复第二个子任务审批。进一步读 Host 与客户端：父 Run `6df1aa20-9979-4e7d-81ad-3708746fec17` 两端均已 sealed，子任务仍 waiting_permission。
- 原始父终结 seal 的 result=incomplete，reason=`execution.no_progress`，detail=`the identical typed tool batch (wait_task) was requested 3 consecutive times; strategy recovery budget exhausted`。这不是客户端自行封存，也不是丢失模型绑定。父任务错误地结束导致只接受活跃父 Run 的审批观察无法继续展示子请求。
- `wait_task_until_return` 遇到 waiting_permission 立即返回 action_required；模型重试等待会触发 Core 相同工具批次检测。后续需修正类型化等待/用户动作的调度契约，避免正常等待触发无进展封存；不能直接关闭一般重复工具保护或仅屏蔽报错，也不能伪造终结结果。CLI 等待时仍须允许即时 /allow 等交互。

### 活跃子任务等待修复与重新验收（2026-10-25）

- Core 的重复批次检测现在区分精确归属当前 session/Run、已登记且仍活动的 `wait_task`。等待外部进展不计为重复失败；未知/外来/终结/正在取消任务仍受保护，混合批次中的普通调用仍检测，纯等待不清空普通调用的历史。既有确定性策略失败保护保留。
- Host 在子任务 waiting_permission 时不再立即返回模型；继续有界等待通知或截止时间，超时返回真实 action_required。取消等待优先处理，不代替用户批准/拒绝，也不取消子任务。没有更改已封存的失败 Run。
- 验证：Core 精确归属单测通过；`agent_task_handle_contract` 13 项、`agent_task_terminal_receipt_contract` 8 项通过（`/tmp/nola-live-task-wait-contract-rerun.log`），含 8 次同任务等待、checkpoint 恢复以及普通工具第 3 次重复仍被阻止。它们是明确模拟 Host 响应的协议测试，不冒充真实模型测试。
- 真实 Store 的生产审批等待用例通过（`/tmp/nola-bounded-permission-wait-stack-test.log`）：至少等待请求的 1 秒、超时审批仍 pending、取消等待不改变子任务，随后原精确审批/恢复链继续通过。第一次默认测试线程栈溢出日志保留；重跑设置 `RUST_MIN_STACK=33554432`，不可称第一次成功。
- 两个 Core 集成测试遗漏的 admission 构造器补齐当前版本常量与 predecessor 字段；不加入旧格式兼容。
- 新产品 CLI 构建通过。隔离 Host 5147 使用用户配置的真实模型，Chrome 真实 56 次操作、模型回复与历史恢复通过：`/tmp/nola-web-live-wait-fix/report.json`。page errors 为 0；网络记录仍有关闭流的 ERR_ABORTED，以及未建立 managed browser 会话时的 409，不将其隐藏或宣称全部 HTTP 成功。
- iOS 新版本完整双 Agent 用例正在 `/tmp/nola-ios-live-wait-fix.xcresult` 运行，尚不能列为通过。此前失败的双 Agent 与冷恢复记录保留。Android、Mac、CLI 此次新补丁后的完整真实操作仍未完成，不能据此声称全平台所有按钮已验收。

- 新版 CLI 真实 PTY 验证也通过：`/help` 实际打开，真实模型回复 `CLI_REAL_BUTTONS_OK`，关闭后以原 conversation ID 恢复并显示原回复。证据 `/tmp/nola-cli-live-wait-fix.log` 与 `/private/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-allplatform-n19vjb5y/`。这不覆盖 CLI 子任务审批交互。


### 第二个实际 UI 缺陷与原会话恢复通过（2026-10-25）

- `/tmp/nola-ios-live-wait-fix.xcresult` 在约 179 秒失败：审批决定已进入发送，卡片仍启用；测试连续点击同一按钮，最后一次在按钮被移除时失败。服务层 continuation 已消费，不是重复执行工具，但 UI 未反映提交中状态。
- `IOSHostPermissionSession` 现在在消费决定时同步发布 submitting；ChatManager 按原 execution 转发精确 request ID，普通时间线与 Room 审批卡共享该状态，提交中禁用全部决定并显示进度。失败后先安装接收器再恢复选择，既有即时重试保护保留。XCTest 也检查 enabled，并在截图后重新确认目标仍可点。
- 在 Host 5147 **同一个原会话**继续验证：`/tmp/nola-ios-cold-wait-fix.xcresult` 成功，日志 `/tmp/nola-ios-cold-wait-fix.log`。冷启动恢复原隔离命名空间，点击第二个子任务的真实审批，两个实际任务分别 sealed 且 result_summary 含 CHILD_ONE=323 / CHILD_TWO=667；父汇总显示并结束。Host 中 Web 验证、父任务和两个子任务共 4 个 Run 全部 sealed。没有重发用户任务、改 seal 或删除数据库。
- 此运行也执行并通过 HostPermissionPresentationTests，包含提交中重复决定不覆盖第一次决定、立即决定与失败后立即重试。完整双 Agent 原单次运行仍失败，后续的四轮对话/手动压缩尚未验收，不把补充冷恢复通过扩大为整套通过。

### 同一审批更新 nonce 的处理（2026-10-25）

- 检查长等待路径发现 iOS interaction 只按 request ID 去重，可能忽略续发的 nonce/expiry 或更新的审阅内容。改为比较完整请求和 conversation；只有精确重放复用现有 interaction，更新挑战取消旧接收器并展示新请求。
- `HostPermissionPresentationTests` 增加生产 interaction 的更新挑战测试：精确重放不再发布卡片，更新 nonce 后旧任务不得发送，只有新 nonce 被提交。8 项测试通过（`/tmp/nola-ios-full-retest.log`）。Android 相应分支本来就比较完整 request；本次未声称完成 Android 真实远程验证。
- 隔离 Host 5148 与新会话的完整 iOS 双子任务、结果恢复、四轮真实对话及手动压缩重跑中，结果 `/tmp/nola-ios-full-retest.xcresult` 尚待确认。

- 完整重跑 `/tmp/nola-ios-full-retest.xcresult` 已失败，不能标绿：Host 配对实际成功；启动后等 app idle 约 350 秒，随后输入原任务时 XCTest event synthesis 超时（715 秒结束），任务尚未发送。此前“连接阶段停滞”的初步判断已被日志否定。失败后 app PID 17707 已退出，无法再取得该进程的 sample；没有伪造卡顿根因。
- 已只重启专用模拟器 A08FB5BF-E10E-4ACB-8922-BC0F1FA906F9，保留容器/数据库；bootstatus 确认启动。同一构建以 test-without-building 重新执行，`/tmp/nola-ios-full-retest-booted.xcresult`，仍用原完成时限。原生 CUA 再次尝试仍报 pipe startup failed，实际点击继续由 XCTest 执行。

### Mac A2UI 重复发布的可复现原因与共享修复（2026-10-25）

- 读取保留的 `/tmp/nola-mac-real-memory.XLeGtb/runtime/ApplicationSupport/Nola/State/nola-v4.db`（只读）：首次 UI action 更新 expected_revision=1 因真实用户编辑推进到 revision=2 被拒绝，模型随后以 2 正确发布 `ACK ORCHID019EBDE4`。后续同内容重复发布均成功，revision 从 3 增至 13；因此之前“内容仍未更新”的推断不成立，核心问题是重复发布持续产生虚假变化并绕过按完整参数比较的循环检测。
- `publish_ui_messages` 在完整协议/归属/版本和投影校验后比较状态（排除候选 revision），无变化返回原 snapshot，不写 Surface、part 或 renderer journal event。`UiToolPublication.changed` 明确表示是否产生新状态；无变化返回原 revision 和明确提示。不得自动封存 Run，也不假造业务操作完成。
- 生产 Core/SQLite `generative_ui_event_permission_e2e` 5 项通过：`/tmp/nola-ui-noop-publication-tests.log`。覆盖两次独立已授权 tool invocation 重复发布、快照/版本数保持不变、后续删除/重建、权限决定、丢失确认和冷恢复；不是实模型通过证据。
- Mac 真实模型/NSWindow 输入与点击重跑已启动：`/tmp/nola-mac-ui-noop-real.log`，Test 配置、隔离 Store，保留正常 Debug 签名应用。结果待确认。
- iOS 重启后 `/tmp/nola-ios-full-retest-booted.xcresult` 再次失败在输入配对 token 的 XCTest event synthesis（约 347 秒）；尚未发送任务。没有第三次盲目重启。之前的真实双子任务冷恢复成功记录仍有效，但本轮完整新建/压缩验收不能标绿。

### 并行收尾：真实 Web Goal 控制复现（2026-10-25）

- 新增 `web/playwright_runtime/real_goal_controls.cjs`：在隔离 Host 的 Chrome 中真实点击新建会话、进度、开始目标、暂停、刷新恢复、继续、取消确认；读取每次命令 receipt，不拦截网络、不直接修改 Store。
- 首次实测失败：`/tmp/nola-real-goal-controls-diagnostic/report.json`，开始目标 HTTP 409，`Host invocation requires an explicit model endpoint`。当前会话已选择导入模型，错误来自 Goal 控制直接使用没有 endpoint 的 Host 启动配置。
- 修复 `control_conversation_goal_response` 复用会话配置继承和模型解析，再使用与 compaction 共用的 `conversation_execution_config`。编译/真实按钮重测尚待完成，不能算已通过。
- 管理页面真实 37 次点击通过：`/tmp/nola-web-parallel-sections/report.json`。真实发送另失败：`/tmp/nola-web-parallel-full-diagnostic/report.json`，180 秒未收到答案，同时 inventory/home 请求 pending；后续独立 HTTP 请求均 0.08 秒完成，尚不能确认 Host 全局锁死。保留失败证据并增加 pending request 路径/耗时诊断，不输出凭据。

- 修复后真实复测通过：`/tmp/nola-real-goal-controls-fixed/report.json`，新构建隔离 Host 5149，Chrome 9 次点击；开始/暂停/继续/取消各 HTTP 200，状态依次 running/paused/running/cancelled，暂停后 reload 持久状态仍正确，0 page errors。两条共用执行配置契约测试在独立 NOLA_HOME 下通过；第一次未隔离运行受用户模型配置影响失败，未隐藏该结果。此证据证明目标控制生命周期，不等于长目标自动执行、多专家或所有平台完成。

### 状态验收修正与同类模型配置入口（2026-10-25）

- Mac `vURj5s` 实际 UI 更新和历史恢复成功，但 action Run最终 sealed/incomplete，不能凭 activeRun=nil认定任务成功。当前测试已强化成功终态断言、修正canonical transcript导出；Test构建通过。模型在无声明资源范围时要求completed被Core正确拒绝，本次未放宽验收保护。
- Web手动压缩入口也遗漏 `apply_conversation_request_configuration`，与Goal相同会绕开会话保存模型；现已先继承会话模型/参数，并将conversationId传给内存策略解析。`/tmp/nola-web-session-controls-check.log` cargo check通过；尚未将该补丁称为真实长历史压缩验收通过。
- 全平台当前状态汇总新增在 `UI_ALL_PLATFORMS.md` 顶部，明确历史批次记录不能代表本轮全面收尾。

### 协议与Goal协调规则收进Core（2026-10-25）

- 新增Core Domain类型SubmissionFrontierRequest/SubmissionFrontier与ApplicationCommand::ObserveSubmissionFrontier；沿用source receipt归属验证和既有原子conversation tail snapshot。Host HTTP只做认证、加密与转发；已有会话必须证明admitted source owner，新Host会话返回明确null/null，不重绑已有提交。
- 生产SQLite测试 `submission_frontier_e2e`通过（`/tmp/nola-submission-frontier-tests.log`）；保留5148原Store/身份重启后，实际加密HTTP已有owner200、错client409、新UUID200，证据 `/tmp/nola-frontier-live-contract.json`。首次手写诊断使用错误加密编码被400拒绝，修正诊断客户端后上述契约通过，未修改服务端加密要求。
- iOS原会话本地configuration_revision=2而Host=1，已确认不能拿本地镜像修补远程precondition；Android也发现相同来源错误。两端按Core frontier冻结接入，后续真实任务尚待新构建。
- 唯一Goal循环迁至 `core/nola-runtime-driver/src/goal.rs`，Host保留typed port适配。Web Start删除自建requirements和固定goal-conversationID，改用Core create_goal_command(action UUID)。`/tmp/nola-core-goal-web-check.log`通过。Web持久worker/观察接入尚未完成，不得把生命周期按钮成功当目标执行成功。
- Goal discovery不再让无lease的running目标固定等待2小时；只读候选立即可见，实际执行仍必须Core claim，活lease/NeedsUser/手动paused仍受保护。边界回归通过 `/tmp/nola-goal-discovery-tests.log`。
- Web Goal面板运行时保留暂停/取消入口，refresh不再清空已显示状态，流式刷新合并；Svelte check与chatActivity测试通过，尚待真实执行中的控制交互验证。

### Goal观察与控制回执接线（2026-10-25，尚待真实worker验收）

- Web读取Core `goal_execution`证明具体Run归属；`goal_control`负责跨轮无Run时的准入事实，不能把空Run等同可提交。Goal聊天停止/恢复走带expectedGoalId/expectedRevision的控制，不冒充remote source receipt。
- HTTP 202仅表示控制排队；面板保留operationId并读取Core applied_operations，直到明确applied/rejected。等待期间显示确认中、禁止重复控制，后台轮询不重复插入加载提示造成跳动。真实Chrome控制脚本同样检查具体操作落库，不能把202算完成。
- Goal权限的执行client与浏览器观察client分别保存，真实挑战仍带原执行client回传；同Run禁止改绑，普通聊天仍拒绝其他client。补22项权限/观察回归通过 `/tmp/nola-goal-permission-tests.log`，Svelte检查通过。
- Web全量494项回归通过 `/tmp/nola-goal-web-regression.log`（在上述权限绑定补丁之前）；此前发现并修复权限刷新fixture遗漏新协议字段，以及资料页非浏览器渲染直接读取location的问题。生产前端build通过 `/tmp/nola-goal-web-build.log`。
- 这些属于协议和组件验证，不等于真实模型Goal持续执行、运行中权限批准、暂停恢复完成；后续必须在新Host上实际点击核验。

### 真实Goal恢复与暂停发现的阻断（2026-10-25）

- 隔离Host5151实际Chrome开始Goal成功，暂停409：Host错误地把AgentSubmission再次包成Boundary Submission再入队。证据 `/tmp/nola-real-goal-worker-controls/report.json`；已改生产调用为当前typed submission，无兼容解析。
- 同Store runs为空但worker反复失败，日志根因是重复启动进程级Runtime Effect Supervisor；已改复用main唯一实例并移除结束后无条件立即wake。为停止失败热循环，确认无active Run后仅停隔离5151，保留其Store/日志；5148真实移动端会话未清理。
- 重启5151后原Goal实际投影paused、runs仍空。整页打开原会话有加载卡住：`/tmp/nola-real-goal-worker-restored-diagnostic/report.json`记录/api/home与静态module仍pending，但同路径独立HTTP读很快返回，尚未定位，不能说Web恢复完成。
- 新生产HTTP fixture又发现首次Goal隐藏continuation没有canonical用户target，以及Guarded Pause取消事实缺原始committed interrupt；这两项正在共享Core修复，尚未通过真实Goal运行/停止复测。
- 配置阻断现通过Core execution_blocker显示安全说明，UI保留明确控制拒绝错误，不在下一次轮询清掉；7项组件/协议测试和Svelte检查通过 `/tmp/nola-goal-blocker-tests.log`、`/tmp/nola-goal-blocker-check.log`。
- Paired compact发现移动端每轮model_intent已冻结、但会话settings没有modelProfileId导致入口拒绝。现复用LoadExactRunInvocation、exact checkpoint证明与Host既有冻结配置重建（同冷恢复helper），不用当前默认模型、不写用户设置；新CLI已编译并更新5148，原会话真实compact仍待验证。

### 当前Host进程身份与共享取消观察（2026-10-25）

- CommittedRuntimeDriver构造/renew不再自行拼PID字符串；Host提供Core typed process identity。CLI macOS读取OS boot UUID和proc_pidinfo创建时间，Linux读取boot_id与/proc starttime；读取失败明确Unknown，不用PID不同、内存registry为空或lease过期代替死亡证明。
- CLI observe_dispatched_target和cancel-only authority统一调用Core InspectCancellationProcessOwners / ValidateCancellationProcessObservations；当前fence、历史target owners、允许以进程死亡确认的请求类别由Core校验。只有Host负责OS观测，实际CancellationConfirmed仍走原fenced响应。
- `/tmp/nola-process-identity-tests.log` 8项通过，含真实子进程创建/退出观测；`/tmp/nola-cancel-process-regression.log` 13项通过、2项外部录制Store测试未运行；`/tmp/nola-runtime-driver-process-tests.log` 全部当前driver测试通过。这些不代表移动端新协议冷取消真实UI已通过。
- Web模块加载无限pending增加30秒显式错误/重试状态；不自动reload、不重新派发业务，迟到module不改变失败结果。4项pageLoad测试与Svelte检查通过；它只解决无限等待的交互退路，未宣称此前偶发整页加载卡顿根因已查明。
# Web 用户等待协议接入（2026-10-25，进行中）

已增加会话 `user-input-waits` GET/HEAD 读取入口，复用既有 Core `ListOpenUserInputWaits` 和 `ListOwnedTaskUserInputWaits`（与 CLI 相同查询）。返回原始会话/Run/wait 身份及子任务 ID，不把子问题改挂到父会话；读取不回答、不授权、不恢复执行。错误使用固定 code，避免返回内部 Store 路径。

`cargo check -p nola_cli --lib --quiet` 已通过，日志 `/tmp/nola-web-waits-check.log`。这仅证明编译；浏览器问答表单、写入与真实等待恢复尚未完成。写入前须补 Core 回答生效后的 Goal 精确继续规则，不由 Web 自行另开 Run。

后续：已补 POST `user-input-waits/respond`，由 Core `RespondToConversationWait` 处理精确 Run/wait/revision/schema/command 身份及重复回答，Host 时间在服务端生成。返回最小的 resolved/conflict 回执，不转发内部执行事件；生效后只唤醒既有 supervisor/Goal worker，未获得 worker 时返回 `wake_pending=true`，不声称已恢复执行。Core 已统一回答后的 Goal 状态变化。`/tmp/nola-web-wait-response-check.log` 编译通过；表单与真实浏览器问答仍待完成。

Web 表单已接入消息区（不是覆盖输入区的浮层），会话与子任务的选项/自由文本/拒绝回答均发送原始 Core owner。网络失败冻结原 command ID 与回答内容供重试；明确冲突要求重新读取问题。读取失败保留已显示内容并禁用提交，卸载中止读取。`/tmp/nola-web-questions-check.log` 0 errors/0 warnings，`/tmp/nola-web-questions-suite.log` 500项通过，其中新增3项覆盖父子归属、重试内容冻结与过期/重复问题拒绝。尚未完成真实浏览器问答、导航往返草稿保留与子任务显示名称验收，不以这些检查代替。

共享冷恢复追加验证：`/tmp/nola-rebind-delivery-regression.log` 的14项 SQLite 派发回归通过；`/tmp/nola-recorded-android-rebind-final.log` 对真实 Android 失败库的只读备份验证通过，含错误 Run/冻结 digest 拒绝且请求未变、重复 prepare 不重复重绑、successor 保留原输入、取消意图重开回执。没有执行模型，也未修改原失败库，不能替代设备实测。


2026-10-26 真实问答追加证据：Chrome 连续实际创建会话、模型主动 request_user_input、刷新恢复问题、选择 Amber 并提交，Core 回执 resolved / wake_pending=false，模型实际回答 QUESTION_CHOICE=Amber，Run sealed/answered。原测试未通过，因为页面持续显示运行中；不能把后台完成当成 UI 验收通过。前台独占复测仍失败，排除并行 Chrome 抢焦点。诊断确认 controller 已 committed，但历史初始化标记仍 false。更深一层是 Core conversation_projection 的 message_state 拒绝当前 writer 正常写入的 waiting_input，消息窗口 HTTP 500；这是共享投影遗漏，不应在 Web 增设状态翻译。

Web 已补：完整 Core terminal manifest 验证与本地恢复准备完成后，可以建立就绪状态，不再由失败的初始历史窗口永久锁住输入。新增 controller 回归验证历史不可用时有效终态解锁、无效 manifest 保持锁定，2/2 通过。尚待 Core waiting_input 修复后同一浏览器身份下真实重测。另一次独立浏览器接管既有会话被 source submission owner mismatch 正确拒绝；保留为跨身份测试结果，不当成本次修复失败或偷偷更改 owner。

证据目录：/tmp/nola-real-question-second、/tmp/nola-real-question-foreground、/tmp/nola-real-question-diagnostic、/tmp/nola-real-question-terminal-fix、/tmp/nola-real-question-terminal-existing。最后两次分别遇到模型提问晚于测试期限、独立浏览器身份不匹配，均未宣称成功。

共享投影修复已落地：当前 `waiting_input` 存储生命周期投影为未终结的 Running 消息；问题本体与精确等待归属仍由现有 user-input-wait 投影提供，不增加各端翻译表或旧格式兼容。`waiting_input_history_projection_survives_cold_open_without_resolving_wait` 使用真实 Core reduction 建立问题、重开隔离 SQLite、执行 LoadConversationProjectionV1，并确认等待记录完整未被修改；1/1 通过（/tmp/nola-waiting-history-core-test.log）。初次测试编译因 StoredUserInputWait 没有 PartialEq 失败，改为序列化完整记录比较后重跑通过。

CLI profile 同步把实际 transport 的显式 reasoning 接收能力与模型 reasoning 参数配置能力分开。Core profile/import 回归实际执行1项通过（/tmp/nola-cli-reasoning-capability-test.log），首次过长模块过滤0项执行不计通过。Web controller 2项回归通过，svelte-check 0 errors/0 warnings。5152确认所有旧Run已sealed后保留SQLite备份并重启新binary，尚待共享模型窗口释放后做最终同身份Chrome问答复测。

当前收敛边界（2026-10-26）：Core负责恢复重绑、取消证据校验、用户等待及回答后的继续、推理流展示策略、终态结果与Goal读取。平台保留OS进程观测、HTTP/SSE及取消执行、IME/焦点/滚动，不从“连接断开/进程registry为空/收到一段正文”推断任务结果。

仍有重复实现需要后续收敛：AndroidCanonicalAgentRuntime.recoverExistingRun与IOSCanonicalAgentApplication恢复循环重复组织inspect/exact invocation/PrepareRunEffectRecovery/schedule/recover_expired/claim；应复用既有Core接口与runtime-driver，不再新增客户端恢复分支。CLI prepare_pending_effect_recovery还有permission重新呈现的特定分支，不能在未区分Prepared/Interrupted前直接替换为批量恢复。客户端异步前后的owner校验、OS停止见证仍有必要，不能为了删代码删除安全边界。Android专家管理当前仅发现编辑器定义而无生产入口，不能将Experts空态点击计为创建专家或多专家任务通过。

最新Web真实重测 /tmp/nola-real-question-core-fixed 未通过：初始历史窗口已HTTP200，但浏览器180秒内未呈现问题；关闭浏览器后原Run已经waiting_input。当前转查HTTP连接/长响应阻塞，保留原Run，不再重复创建模型请求。此结果不能否定已完成的Core状态读取修复，也不能算UI闭环通过。

最终Web问答真实闭环已通过（/tmp/nola-real-question-http-fixed）：Chrome实际连接、新建会话、发送问题任务、模型request_user_input、刷新恢复卡片、选Amber提交、显示QUESTION_CHOICE=Amber、恢复发送按钮，再发送同会话后续任务并显示QUESTION_FOLLOWUP=842。无页面错误；运行时报告验证首Run sealed/answered，事后独立只读Store核对两个Run均sealed/answered，证据post-run-store-proof.json。已人工查看followup.png，运行占位和Stop均已退场。脚本随后补充第二Run自动Store断言，语法检查通过；本次既有报告不冒称执行过后来新增的断言。

HTTP原因由vendored tiny_http TaskPool突发连接容量计算遗漏触发；修复及确定性旧红新绿由Mac子任务验证，新Host真实Chrome只读60请求通过。此前所有失败目录继续保留。浏览器恢复测试状态仅保存到临时目录0600文件，不提交token、数据库或模型配置。

共享恢复协议收敛（2026-10-26）：CLI 已删除自行决定 ResumeHostEffect 的恢复分支，统一调用 Core PrepareRunEffectRecovery。Core 返回当前版本的结构化回执 rebound_count / traces；Apple、Android、CLI 和测试调用方同步更新，不保留旧数字回执兼容。恢复策略只在 Core planner 中定义，Runtime 使用既有 active-submission unit of work 更新热态 supervisor，权限重呈现使用 Core 返回的原始 trace，不由客户端重造。平台仍负责刷新实际 worker 身份和展示。

CLI 权限/Goal 恢复回归26/26通过（/tmp/nola-cli-shared-recovery-regression.log），包括 Prepared 和 live model 连续恢复不重绑。中间两轮各24/26暴露热态 cursor、物理身份刷新和权限 trace 丢失，修复后完整重跑通过，未删除原断言。真实 Android 失败 Store 的隔离备份恢复测试1/1通过（/tmp/nola-shared-recovery-recorded-final.log）；此测试不发模型请求，不代表移动端点击验收。Core Resume 完成后使用正常 checkpoint baseline，流式增量写入仍保持原规则；既有3577条重复恢复记录不通过SQL删除或转换，原5151任务正在走正常取消收尾。

2026-10-26 取消持久边界修复：原5151长tail的单次取消意图已提交，但后续取消查询/回滚仍需重放历史，已观察停止租约在处理过程中到期，整条收尾尚未通过。Core 已将 Interrupt 与 ResumeHostEffect 一样作为低频正常 baseline 边界，防止新取消继续携带整个流式 replay tail；不改变高频模型增量写入，不删除历史或延长租约。nonterminal_replay_e2e 两项冷取消测试通过（/tmp/nola-interrupt-baseline-test.log），同时验证当前baseline无待重放输入、revision相同、原effect目标和冻结prompt仍保持，之后仍需真实停止确认才封存。原已提交任务不会因源码修改自动获此修复；其现存数据继续正常处理，不用SQL改写。

取消边界追加回归13/13通过（/tmp/nola-interrupt-cancellation-regression.log）。首轮12/13，原测试禁止取消写任何checkpoint body，与新低频baseline策略冲突；调整为控制查询仍用会报错的body view证明不读完整body，执行Interrupt前恢复可写表，断言baseline确实变化。原exact owner、过期响应拒绝、receipt重放及迟到响应不能恢复旧状态的断言保留。冷取消2项另验证baseline清空replay tail。原5151收尾实测最终失败：取消receipt提交后，确认响应到达时lease已过期333秒，Core正确拒绝；证据/tmp/nola-exact-stop-final/，原Run未封存，没有新模型调用。不能把本次新边界测试说成旧Run已取消成功。

Goal 控制协议收敛：GuardedControl 改为 expected_goal_id + expected_control_token。Core 持久 control_generation 并生成 opaque token，UI和Host不得自行计算。heartbeat继续更新完整revision供审计，但不使控制token失效；生命周期、执行contract/Run变化及语义工作更新推进控制代次。Pause→Resume即使重新回到Running，旧token仍被拒绝，重复operation仍重放原回执。Core19项通过（/tmp/nola-goal-control-token-tests.log）；Web17项及类型检查/生产构建通过，CLI编译通过，新wire与真实端到端仍待验证。当前GoalProjection要求control_generation，不引入缺字段默认值、旧协议别名或数据迁移；原5151和iOS旧录制数据保留不改，不能宣称新版已成功恢复它们。

Goal 控制边界最终补测26/26通过（/tmp/nola-goal-control-final-boundaries.log），覆盖普通进度不改变token、新Run绑定改变token、heartbeat后冷序列化仍可暂停、Pause/Resume后旧控制拒绝且operation重复结果不变。新增Run绑定用例首轮fixture仍引用旧requirement id失败，改成实际fixture的r后整组重跑通过。另发现Web POST同步Goal回执漏control_token，已交由统一response helper修复，不能仅凭GET测试判定完整接口完成。

取消恢复复用已完成定向17项回归（/tmp/nola-cancellation-hot-final.log、/tmp/nola-cancellation-hot-owner-test.log）：Core RuntimeSupervisor复用同事务中匹配Run/会话/revision的恢复状态，取消证据仍重新核对Store当前owner、租约及冻结配置，客户端不缓存这些事实。256条持久delta的当前格式隔离fixture测得冷取消57.4ms、后续8次证据读取合计9.8ms；这是该fixture的观测，不等于原3830条失败库已收尾。首次回归发现无replay tail的正常baseline不携带invocation，被新增校验错误拒绝，按现有恢复合同修复并全组重跑。当前生产Core冻结，统一CLI构建后做新格式真实Goal控制。

当前格式真实Chrome Goal控制验收未通过：新隔离Host5153、会话conv-20261026102728667-1开始Goal HTTP200，回执含当前control_token；120秒未观察到provider输出，Run goal-run-b6d9586b-0e54-40e0-8121-63b4092ba410仍running。刷新后真实点击暂停返回冻结Host invocation不一致，未改变原Run或重建数据。证据/tmp/nola-real-goal-token-final和/tmp/nola-real-goal-token-control-only。正在定位Goal首次配置与恢复配置差异；不能将17项取消性能测试或26项Goal控制测试算作此实际闭环通过。

子任务交互追加缺口：Web TaskActionSubmission在派发前持久unknown，回执丢失后禁用控制，但只保存状态、不保存可查询的指令身份；task/message服务端现有queue_agent_guidance负责生成message_id。这能避免盲目重复发送，却不能完成不确定结果恢复。下一步须复用Core Task guidance admission/receipt与固定消息身份，保持同指令内容及原parent/task归属；不得仅加UI重试或把断网当作未发送。当前仅确认缺口，未完成实现/实测。

5153失败环境证据补充：pmset日志确认2026-10-26 10:31:19进入Maintenance Sleep154秒，10:34:24再次Sleep155秒，10:36:59 DarkWake。实际Goal窗口有主机睡眠，不能将120秒无delta简单归因模型/网络延迟。Host另记录effect :10响应到达超过lease72.7秒；恢复暂停仍有冻结配置不一致错误，睡眠不免除该产品恢复缺口。

5153回执动作纠正：/tmp/nola-real-goal-token-control-only/report.json clicks为连接/进度/继续，HTTP409对应继续，并非暂停。只读最新checkpoint显示Goal running、无blocker、contract executing、Run awaiting_model；具体UI观察与durable状态需分别记录。已定位配置差异：worker用web_goal_invocation冻结权限profile、工具allowlist等；控制入口曾用仅组装模型参数的conversation_execution_config。正在统一当前调用者，不放宽Core exact witness。真实Goal脚本新增仅随测试进程存续的caffeinate idle断言，不修改系统电源配置，不能防止合盖。

子任务指导回执恢复已开始落地：Core新增只读LoadTaskGuidanceReceipt(task/message/parentConversation/parentRun)，按immutable admission核验，允许终态后查询原记录；不存在与查询失败分开。SubmitTaskGuidance重试先核验原content/time/parentTurn并返回原record，不重建当前水位或重复入队。task_run_binding_e2e指导相关4项通过（/tmp/nola-guidance-current-regression.log），含冷打开/原Run完成后查询、原消息重复、内容和时间冲突及外来parentRun拒绝；HTTP/UI与原生调用者正在同步，尚未端到端验收。

Goal冻结配置修复后保留原库重启5153，真实Chrome继续出现20秒POST挂起（/tmp/nola-real-goal-token-fixed）。sample证明Resume直接acquire_core_agent_host等待模型流持有的Mutex；Pause/Cancel此前走持久队列，Resume不同入口是生产缺口。准备将active Goal控制统一入现有AgentCommand队列，回202仅表示排队，原operation回执确认实际状态。原失败任务/请求保留，不因观察超时重发。

Goal队列严格HTTP回归先出现202后没有applied_operations回执，确认有效token但不适用当前生命周期（例如running时Resume）在Core返回Err阻塞FIFO。已在GoalProjection.apply为允许的GuardedControl语义失败记录原子rejection，丢弃临时状态；不支持的控制类型、operation冲突及非guarded错误仍拒绝，不吞Store错误。Core27项通过（/tmp/nola-goal-semantic-rejection-tests.log），含拒绝后同operation重放不改变结果、后续合法Pause可执行。HTTP严格队列测试继续，不能将202作为已完成控制。

指导回执三次重交新增队列计数断言通过（/tmp/nola-guidance-receipt-no-duplicate.log）：原Run完成并冷重开后仍返回原记录，不增加agent_commands。Web/Android已接完整intent，但Apple/mac尚在更新；取消动作回执丢失的unknown恢复仍是独立未完成项。

### 2026-10-26 子任务取消回执丢失后的 Web 状态收敛

- 发现：取消 HTTP 丢响应后，即使随后收到同一任务的 Core journal `cancel_requested` 或终态，Web 控制条仍保留未知/等待取消提示。
- 修复：持久取消意图记录完整 Host、conversation、parent Run 和 task 身份；只由匹配身份的 Core publication 将未知收敛为等待取消，终态清除过期操作提示。能力消失不算取消确认，也不会自动重复发送取消。
- 竞态：Core 终态先于 HTTP 失败到达时，后到的传输错误不能覆盖已观察终态。共享任务渲染器同步刷新控制状态。
- 验证：`node --experimental-strip-types --test web/frontend/src/lib/chatTaskActions.test.ts` 11/11；`npm run check` 0 errors / 0 warnings，日志 `/tmp/nola-task-cancel-observation-check.log`。
- 范围：这是生产控制器与 Core journal 事件的回归，尚未完成真实网络故障下的浏览器点击验证。若请求未进入 Core 且没有后续取消事件，仍保持未知；不伪造确认。

### 2026-10-26 原 Goal Store 冷恢复复验（未通过）

- 当前 CLI 构建完成：`/tmp/nola-goal-final-shared-build.log`；正常 writer 128 条 checkpoint 预算及 exact trace replay 已在同一构建内。
- 已对原隔离测试 Store 做 SQLite backup，使用同一数据库重启 5153 Host（PID 27784），没有清库或新建目标绕过原现场。
- Chrome 实际登录后等待会话按钮 20 秒超时：`/tmp/nola-real-goal-bounded-recovery/report.json`，不能宣称 Goal 暂停/恢复/取消通过。首次合法长尾仍需完整重放，观察超时不视作服务停止。
- 重启前额外发现持久 receipt 计数：prepare_run_effect_recovery 12765 次、claim_committed_host_requests 12822 次；最新约150ms重复一对。正在定位无工作恢复轮询，避免把检查点预算误当作全部根因。
- Mac 当前完整 Test 构建及共享 Apple guidance XCTest 4/4 已通过；这不替代上述真实 Goal 验证。

### 2026-10-26 只读重复恢复与真实导航门槛

- 当前5153 service PID27823 CPU采样 `/tmp/nola-current-cold-service.sample` 79–99行证实：只读 `execute_application_command` → `load_current_checkpoint` → `materialize_replay_checkpoint_json` 执行完整tail重放。不能将该现象解释成仅首次active submission恢复。
- `load_current_checkpoint` 绕过已有 `cached_read_checkpoint`；正在统一入口复用既有精确缓存，保留外部写入/事务回滚失效约束。
- 修正浏览器诊断：出现“进度”工具栏不代表会话已加载；必须有可用会话按钮且不再显示加载状态，失败返回非零退出码。
- 严格导航复验退出1：`/tmp/nola-goal-strict-navigation/network.json`，实际页面尚未就绪。没有再次提交Goal控制。
- iOS实际原故障库副本的exact trace跨FFI恢复已原生通过13/13：`/tmp/nola-ios-recorded-trace-replay-verified.xcresult`；这仅证明恢复事件链，不替代Goal真实控制闭环。

### 2026-10-26 过期租约空调度根因

- 同一当前版本Host再次出现70–150ms间隔的空prepare/claim，非旧二进制遗留。Core schedule给出 `RecoverExpiredLease`，CLI worker却直接warm drain；prepare只处理Interrupted，claim不能领取仍处于过期execution_started的delivery，因此空返回后反复重排。
- 修复方向：消费Core明确给出的reason，在精确Run/冻结配置验证下调用现有 `RecoverExpiredRunDeliveries`，再重新观察Core状态。过期不等于外部副作用已停止；继续沿用Core对Interrupted/Unknown和可恢复性的判断，不放宽安全规则，不加固定sleep掩盖。
- iOS `IOSCanonicalAgentApplication` 与Android `AndroidCanonicalAgentRuntime` 已有该reason分支；此次是CLI遗漏，Mac/Web的本地Host也受影响。
- 共享检查点读取回归最终14/14通过：`/tmp/nola-checkpoint-read-cache.log`。包含Goal读取复用、同baseline下新replay head使缓存失效。真实原会话完整闭环仍待验证。

过期租约修复验证：CLI真实SQLite回归 `scheduled_expired_model_delivery_recovers_rebinds_and_becomes_claimable` 1/1通过（1.68s，`/tmp/nola-expired-claim-test.log`）。实际领取并进入ExecutionStarted后让100ms租约过期，再走生产调度恢复helper、Core prepare及successor claim，确认effect不变而请求/attempt更新。此测试未调用模型；原Goal会话真实操作待新构建复验。

### 原Goal新构建真实点击推进（2026-10-26）

- 同DB备份后启动Host33388，严格Chrome导航通过：`/tmp/nola-goal-recovery-navigation-final`。
- `/tmp/nola-real-goal-recovery-final/report.json`：继续202（1430ms）→精确operation ledger running；暂停202（2862ms）→ledger paused；原Run封存。刷新后第二次继续409，响应Goal running/control state changed，完整闭环失败，继续诊断。
- 证据纠正：此次脚本realModel字段引用原Run已有707delta/84completion，不能证明恢复后产生新输出。已修脚本在控制前保存计数，同Run恢复必须计数增长；后续真实验证用此更严格门槛。本轮只确认操作回执和暂停封存，不声称新模型输出或完整取消闭环通过。

### Goal 暂停后被后台自动恢复的精确根因

Core操作账本：用户Pause rev98后，出现内部resume rev100、interactive owner claim rev101，用户刷新后第二次Resume才在rev102被严格拒绝。后台`web_goal_invocation`恢复原运行的GoalInteractiveUser输入来源，HostGoalPort却据此重新判断本次执行为interactive，触发自动Resume。修复为本次调度方式不可变，Web后台显式scheduled，恢复冻结输入不改变调度方式。保持Core token/claim约束，不以自动重试409掩盖。另检查恢复失败分支是否可能给已Pause的目标重新安排唤醒；同属暂停语义保护。

### Goal 控制 epoch 与内部执行进度分离

- `/tmp/nola-real-goal-dispatch-final/report.json`验证新Run真实model completion，但Pause在队列应用时因内部推进改变control token而拒绝；完整流程未通过。
- 修复token语义：仅可控制生命周期变化使control_generation失效。内部contract/report/audit/Run绑定和work更新仍递增各自revision，但不使Pause/Cancel失效；claim/release仅改变状态、停止原因或唤醒时间时改变generation。
- Goal/work回归28/28通过 `/tmp/nola-goal-control-lifecycle-tests.log`；保留Pause→Resume ABA、跨Goal和exact Run owner约束，内部完整round和worker交接后仍可暂停。
- 当前测试任务已通过真实Chrome停止：`/tmp/nola-goal-owned-stop/report.json`显示authoritative paused；SQLite确认Run95ca235a-f9c5-4405-a987-62afd4e95d65 sealed/cancelled。此为清理当前测试执行，不替代完整重复控制验收。

### 新目标 Pause 在未启动模型边界卡住

`/tmp/nola-real-goal-lifecycle-ui-final/report.json`：Start200后Run835755a2真实model completion，Pause202且操作账本paused，但60秒Run未封存。只读Store显示active_turn=cancelling；model effect703已settled并有rejected_before_execution/cancellation_not_started_admission，cancel706携带target_not_started证明但尚pending。Host错误为`hot cursor belongs to a different committed Host delivery`。

根因：Store原子取消给outbox补入合法target_not_started证明，hot reducer仍保留None；`overlay_request_cursor`未应用已有CoreHostBindingOverlay即全字段比较，错误拒绝领取cancel。计划复用现有`HostEffectControl.apply_binding`单调证明补全后继续严格身份/字段比较，验证未启动模型取消的真实hot Store路径及伪造/替换证明拒绝。不得将目标已暂停当Run已停止。

### Goal 原现场完整真实浏览器验收通过

- 最新取消证明修复启动后，原卡住Run835755a2自动sealed/cancelled，没有清库、直接改状态或重复发停止。
- `/tmp/nola-real-goal-proof-final/report.json`：8次真实Chrome按钮点击，继续→暂停→刷新→继续→确认取消全部通过，零pageerror；每个202均查询同一operation ledger确认，最终页面目标状态已取消。
- 两次新增真实provider completion对应Run e8f5703c-c5e5-48bd-9568-d3963d7a45ac与a77d2464-5c7c-44d6-adf1-fc428ed9ff54，独立SQLite结果均sealed/cancelled，见`terminal-proof.json`。未将历史输出当本次恢复成功。
- 已查看`cancelled.png`确认目标状态。红色「处理过程有待核对」经实际 Store 核对来自先前 e8f5703c Run 的 goal_update 工具失败（tool_call:717）：模型提交了 Core 专属 provenance 字段，返回 `Goal record provenance is assigned by Core, not the model`。不是取消状态误报；保持错误可见，不以隐藏警告通过验收。后续 a77d2464 Run 的 goal_update 已成功。
- 已停止不再使用的隔离提问Host5152以减少后台CPU竞争；停止前备份Store，原sealed/waiting_input数据均保留。5148、5153、5154当前验收Host未因此停止。
- 当前仍未通过：子任务运行中页面及时加载/状态刷新/指导消息，以及iOS远程列表未确认回执提示。独立代理正在真实验证，不以Goal通过替代这些验收。

### 子任务运行中权限读取复用与验证边界

- `ReadPermissionConfiguration` 在一致性读内复用 RuntimeSupervisor 的已提交权限投影；仅精确 Run/conversation/checkpoint revision 且无未提交 deferred 事件时命中。保留原 owner、remote admission、policy revision 校验；未命中仍走权威 Store 读取。
- 权限 configuration snapshot 3 项、projection read 11 项、2MB Store hot/cold 1 项和 supervisor exact/deferred 1 项通过。hot/cold 的耗时仅是 fixture 内路径测量，不能作为真实浏览器加载通过证据。
- `/tmp/nola-permission-hot-cli-build.log`：统一 CLI 构建成功。5154 同一隔离 Store 备份并优雅重启，真实指导消息丢响应恢复正在运行，证据目录 `/tmp/nola-real-web-guidance-recovery`；尚未判定通过。

### 新建会话草稿归属真实回归

- 点击新建后旧 composer 在异步切换期间仍可输入，文字会写入旧 owner，随后被新会话 snapshot 替换。现创建/选择期间统一锁定输入、发送和附件，键盘指令入口也检查同一状态；选择序号防止旧异步完成解除新选择的锁。
- `/tmp/nola-real-draft-switch/report.json` 真实 Chrome：新建后立即尝试输入时 disabled=true；两个会话草稿独立保存，刷新和返回后恢复；最终 sendEnabled=true，errors=[]。本测试未调用模型，不据此宣称消息发送链路通过。
- 子任务指导仍未通过：真实 POST 返回 `guidance task has not started a bound Run`，旧 journal producer 却无条件发布 Agent supports_guidance=true。正在以 Core 实际 task/Run binding 发布能力，不能由客户端猜测 running 状态。

### 历史子任务详情入口真实验证

- Web 任务展开复用现有 messages API 的 Core taskProjection，校验 task id、原父会话和原 Run，再使用 lineage.child_session_id 打开子会话；不解析 nola-journal 指针猜测归属。
- `/tmp/nola-real-task-details-fixed/report.json` success=true/errors=[]：从历史父会话真实点击任务入口，进入 `agent-4ed50debe3184f42` 后等消息正文显示真实结果2701，主代理已查看 `child-result.png`。没有以侧栏父提示中的标记判定成功，没有新增模型请求。
- 新发现仍需处理：子会话详情仍展示普通 composer 和模型选择，而 Core 的 task-child 准入有明确专用入口；正在核对可复用的 Core 子任务归属投影，以免显示不可执行操作。此项不影响已证明的结果阅读，但详情交互尚未完整验收。

### 父子任务 journal 实时通知隔离

- 子 Run 准入只返回自身 publication cursors；父 Task guidance metadata 与真实binding在同事务提交，但不把父cursor送入childconsumer。
- 新增 source `assistant_journal_wait`，复用既有 Service commit wake socket 和 disconnect-aware wait；只读指定完整journal identity，拒绝foreign/ahead/equal-sequence不同cursor，不读累计文本，不写待恢复命令。Host普通流与权限恢复流从首次已准入的本Run journal cursor启动等待，返回的hint保留同owner；关闭stream主动cancel socket。
- `/tmp/nola-journal-wait-ipc-tests.log`：实际Service进程+SDK+SQLite原guidance/restart用例通过，新增断言提交唤醒、foreign Run拒绝及30秒等待主动取消在2秒内结束。不是模型/浏览器验证。
- `/tmp/nola-epoch6-journal-cli-build.log` 统一CLI构建通过。新增source stamp与远程归属持久化使用当前epoch6，旧测试库不迁移、不清理；5155新隔离Host用于真实验收。旧5154父Run非终态仍保留，不能把新库通过当旧现场已恢复。
- 尚未通过：新Host父Task无需刷新false→true，以及真实指导已提交响应丢失后同intent恢复并delivered；iOS本地embedded父journal更新也未据此验收。
