> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Harness / prompt 链路审计（2026-10-13）

状态：源码比较与真实轨迹支持的整改设计；不是全链路验收通过。
后续全链路审查发现参数身份、delegated 回执、截断提示、请求快照与媒体投影仍有不一致；下述分轮测试不是当前全链路完成证明。当前整改及最终验收见 [全链路契约收敛](../实施与审计/HARNESS_CHAIN_CONVERGENCE.md)。
审计起点为 deadline-v3 四题运行期间；随后 harness-v4 四题已结束。每批 Linux 二进制保持冻结，不能用旧二进制结果证明批次结束后新代码有效。

## 深审后的统一边界整改

本节覆盖旧版推断，不将此前通过的测试算作本次验收。四题并非同一个失败原因：music 重复提交相同内容；freight 的承诺状态与多阶段自测缺失；photonic 的验证器和候选都错误；FreeCAD 始终停留在观测并固化了误读。

### 模块输入、输出与所有权

| 模块 | 输入 | 输出与保证 | 不保证 |
| --- | --- | --- | --- |
| Host 执行证据 | 具体调用、受权资源、进程结果 | 资源前后版本，ProcessExit 或 Postcondition 类型的检查及明确谓词 | exit=0 不证明用户需求满足；未知版本不冒充变化 |
| Core 工程过程 | Host 原始收据、当前资源版本、恢复预算 | 对应版本的执行检查债、AwaitingAssessment 阶段、有界恢复 | 工程阶段不是任务终态；不能由写入成功次数证明进展 |
| Goal 验收 | 明确验收条件及匹配证据 | required check 的 kind/资源/版本/谓词精确匹配；执行要求可由 ProcessExit 满足 | 任意脚本成功不能满足声明为 Postcondition 的领域条件 |
| Context | 追加式原历史、近期原文、之前摘要、可信当前运行状态 | 热分叉或独立摘要路径共享事实约束，摘要为低权威历史上下文 | 摘要不能授予权限、重设剩余预算、确认未观察的产物 |
| 评测归档 | 隔离数据库、事件和输出引用 | 完整输出文件、调用关联、原始/归档哈希、脱敏及缺失状态 | preview 不是原始结果；仅有引用不是已归档 |

同内容 write_file 返回 changed=false，不重写、不运行格式化、不制造新 artifact。真实 write/edit 的 before/after 来自 Host 文件观察，不从模型文字推导。ProcessExit 与 Postcondition 是必填不同类型，不保留旧检查格式的兼容默认。工程阶段线格式删除 ready_to_complete，替换为 awaiting_assessment；checkpoint 同步升级。

### 验收与性能约束

- 同内容重写不产生新修改信用；真实版本变化可产生进展，但不等于正确。
- 正常修改→执行对应版本测试→评估结果链必须可达，不制造没有任何工具能满足的永久验证债。
- 精确重复拒绝不剥夺合法替代工具；恢复预算仍有界，自动批准不自动增加额度。
- 摘要保留失败/未验证/推测标记；每轮预算和权限来源是实时运行状态，不是摘要里的旧数字。
- 评测 supervisor 注入实际 deadline 和时限，Host 在任务模型请求尾部投影带 observed_at 的剩余时间；无显式外部预算不虚构时限。内部 provider 重试复用该请求快照，下一任务请求刷新；实际终止仍由 supervisor 执行。
- 归档完整性以引用计数核对；缺失显式列出，不因导出失败生成空的“成功证据包”。原始凭据不进入新增输出归档。
- 结果归档只在评测收尾执行，不增加每 token 工作；内容相同的归档按哈希去重。正文投影仍有限，完整结果按需回读。
- 整体代码与静态审计结束后统一测试；真实模型分数必须来自新冻结二进制，旧 v4 分数不证明改造效果。

### 本次统一验证结果

全部实现及交叉审计后统一测试，结果如下（2026-10-13）：

| 范围 | 通过 | 忽略 | 证据 |
| --- | ---: | ---: | --- |
| Core / Domain / Host protocol 库 | 505 / 170 / 28 | 0 | `/tmp/nola-harness-overhaul-verified.log` |
| CLI 库 | 1787 | 4 | 同上 |
| Application / Store 库 | 315 / 409 | 1 / 1 | 同上 |
| Core→Host 契约集成 | 145 | 0 | `/tmp/nola-harness-overhaul-host-contract.log` |
| Application nonterminal 冷恢复集成 | 7 | 0 | `/tmp/nola-harness-overhaul-replay.log` |
| 归档 / Harbor / supervisor Python | 8 / 6 / 3 | 0 | `/tmp/nola-harness-overhaul-{export,harbor,supervisor}-verified.log` |

Rust 合计 3366 通过、6 忽略；忽略项不算通过。Mac 原生桥接文件通过 `swiftc -frontend -parse`，不等于 Xcode 全量构建或真实 Mac 会话验收。`git diff --check` 通过。

保留首次失败，不以复跑覆盖：`/tmp/nola-harness-overhaul-rust.log` 的 CLI 续写 fixture accepted socket 在非阻塞监听后未切换为阻塞读取，出现 WouldBlock；修正 socket 模式并增加诊断，未扩大生产超时。`/tmp/nola-harness-overhaul-final.log` 的 Application 旧正例要求输出占满总窗口4096，与现行保留输入空间契约冲突；正例改2048，并新增4096明确拒绝断言，未放宽生产规则。首次 Python 模块式命令受环境包名冲突失败，最终使用 unittest discovery 验证。

没有重跑新二进制的 LAN 四题，不能宣称任务成功率或 prompt-cache 命中率提升。运行预算快照标明 observed_at，provider 内部重试复用该快照、下一任务模型请求刷新；原始资料中遗漏的 stdout 不能由新归档器追溯补造。

## 本轮实现进度（四个子代理分工）

- 恢复控制：保留合法替代工具；Core checkpoint 20 保存拒绝指纹、已消费恢复请求及来源观察见证。配置 `max_strategy_recovery_attempts` 默认 3、0 禁止自动恢复；冻结配置缺字段拒绝恢复。全局普通迭代无限不代表恢复无限。Host 契约 144 项通过（`/tmp/nola-harness-core-integration.log`）。
- Provider：稳定 system/developer blocks、最后内容 block 的缓存标记、单消息分叉不写新尾缓存；移除每轮给旧工具结果补 cache_reference。动态 system 的历史缓存问题尚未闭环，详见 [协议指令投影](../../软件架构设计/模块/08-模型配置与调用/PROVIDER_INSTRUCTION_PROJECTION.md)。
- 取消：JSON、TTY、native 入口共用可清理信号适配器，只发取消请求，不凭进程退出制造 seal。隔离真实 JSON CLI + localhost provider + SQLite 验收 3/3：SIGINT/SIGTERM sealed/cancelled，SIGKILL 无 seal（`/tmp/nola-harness-signal-acceptance.log`）；不代表 native 授权等待或工具进程树取消已全覆盖。
- Prompt：Host 只保留环境和呈现事实；Core 统一行动范围，普通任务内必要可逆步骤无需额外点名，但仍逐调用核验权限；可选 terminal SOP v3 不强制短任务维护台账。

上述不是 HCTRL-11 全完成：缺 mutation/validation 版本化进展见证、资源版本变化后的精确重新准入、完整 tagged recovery decision。观察变化只证明新观察，不证明语义正确或要求满足；没有真实 provider 命中率提升证据。

### 集成验收账本

- Core 库 495 项、Host 契约 144 项通过；真实取消进程 3 项通过。
- CLI 首次默认测试栈溢出；16 MiB 栈全量首次 1767 通过、12 失败、4 忽略。已修共享 fixture 目录/数据库、旧 fixture 状态和 lease、音频能力说明回归。
- 全量复测 `/tmp/nola-harness-cli-final.log`：1773 通过、7 失败、4 忽略；7 项来自 fixture 写入和读回使用不同隔离 store，现已改为显式共享实例，不放松生产隔离。
- 工具专项 `/tmp/nola-harness-tools-final.log`：442 通过、2 失败，之前 7 项已通过。剩余 `auto_mode_promotes_one_process_to_a_durable_background_task` 与 `completion_notification_is_acknowledged_across_process_reconstruction` 报任务记录消失，尚未归因；需要检查后台任务存储与测试环境作用域，不能直接认定偶发或重跑即解决。
- 上述两失败已定位明确污染入口：`ResourceTestEnvironment` 修改 `NOLA_DB_PATH` 却未取得统一环境锁。任务创建/启动/查询与通知确认分别解析默认存储，因而可读到另一个 fixture 的数据库。已将锁归 RAII 环境所有者持有，覆盖保存原环境、设置与恢复，移除调用方重复锁防死锁；未吞掉 missing-task 错误或修改生产权限。复验日志 `/tmp/nola-harness-tools-isolation.log`。
- 隔离修复后的工具专项 444/444 通过（15.77s）；完整 CLI 最终复验 1780 通过、0 失败、4 忽略（118.88s），日志 `/tmp/nola-harness-cli-verified.log`，使用 16 MiB 测试线程栈。随后 harness-v4 四题结束：三个零分、一个评测异常。忽略项不是通过；默认栈和大测试二进制 unwind 警告不能记为已解决。
- harness-v4 真实第二题否定了“恢复收尾已纵向闭环”：从 tool_completed 进入预算耗尽收尾时，Store 拒绝结果来源（不是原始已完成模型请求），没有 seal。深审纠正：music 的 seq68/123/163 及 seq181 请求内容完全一致，不能据此声称真实修改后复验被误拒。资源变化后的重新准入应独立用反例验证。Core/CLI 单测绿灯未覆盖该 Application 提交链，不能删除 Store 来源校验或把未知/失败写成 Incomplete。
- v4 结束后开始下一轮整改，先统一改代码、审计后测试。容量/截断拒绝不再强制把 `write_file` 或任意 `bash` 替换为 `append_file`；恢复状态与原权限仍保留，模型自行选择完整合法的替代策略。本轮独立验证结果见下节；不可用上面的 v4 数字作为新代码通过证据。

### v4 后整改及真实提交回归

代码已补齐容量拒绝的工具和提示词契约：不强制 append，执行预留轮也不因没有指定 recovery_tool 就剥夺自主纠错能力。Host 的文本、JSON 序列化后内容、追加内容统一使用 500000 UTF-8 字节上限；不再把 JSON Schema 的字符 maxLength 当字节上限。完整文件替换复用原子写入，保留模式和路径授权。append 本身仍非幂等事务，提交后格式化也未声称属于同一个原子事务。

停止结果新增 RecoveryStopped 事实，区分预算耗尽、空续行和未形成结果的控制调用；保留旧 Model 来源严格校验，Runtime 停止必须引用当前原始 transition 的事实。Store 约束停止事实、Runtime execution.no_progress 来源和 Incomplete/execution.no_progress 结果三者一致。

第一次完整提交回归仍失败：Core 账本事件已落库，但 Application 没保存它的 core_envelope，Store 无法确认同一 revision/cursor。已为 Pending 恢复账本事件补来源前沿；没有改用最新 checkpoint 取证，也没有为所有普通工程事件增加前沿或再复制状态。新增 Application 测试用真实 effect lease，每一边界冷打开并重复投递，覆盖额度 0/3 和最后工具成功/失败。

统一复验结果：Core 496/496、Host 契约 145/145、Store 停止来源负例 2/2、Application 来源前沿 4/4、Application 冷恢复/重复投递 7/7，日志 `/tmp/nola-harness-v5-*-verified.log`；工具专项 446/446，日志 `/tmp/nola-harness-v5-tools.log`。Core/Host 的旧提示词断言与重复注册 fixture 已修正；Application 的第一次失败及上述根因仍保留在 `/tmp/nola-harness-v5-application-final.log`，不能只保留成功记录。Host 容量恢复测试同时覆盖无限迭代和进入预留轮的配置。

这些不是新版本 LAN 模型验收，未重建或重跑本轮 v5 Linux 四题，不能声称得分提升。剩余契约缺口：逐消费序号对应独立请求 ID 的证明、资源依赖版本化重新准入、保留安全模型失败阶段诊断；均不得记为已完成。EmptyModelContinuation/UnfinishedRequirementControl 两类原因尚缺同级 Application 正向封存用例；当前真实落库回归主覆盖预算耗尽。

## 比较范围与证据边界

- pi：`参考资料/pi/packages/agent/src/agent-loop.ts` 的模型请求、工具结果、下一轮、取消流程；`packages/coding-agent/src/core/system-prompt.ts` 的提示词组装。
- OpenCode：`参考资料/opencode-dev/packages/opencode/src/session/processor.ts` 的工具生命周期与重复调用处理；`session/prompt/default.txt`、`tool/bash.ts`。
- Claude Code：`参考资料/claude-code-system-prompts-main/system-prompts/` 中 harness、doing-tasks、delivering-work-at-full-scope 提示词。它是提示词收集资料，不是已验证的 Claude Code 完整运行时源码，不能据此断言其内部机制。
- Nola：Core prompt composer / model planner / submission / engineering state，Host prompt overlays，provider protocol conversion，JSON CLI cancellation entry；真实证据见 `evals/benchmarks/terminal-bench/LAN-20261013.md`。

## 实际链路与所有权

用户任务 → Host 环境/工作流片段 → Core 政策与运行状态 → 本 Step 工具投影 → provider 请求体 → 流事务 → 工具准入/执行/观察 → Core 下一步/终态 → Application 原子提交。

必须沿整条链判断，不能由某一层的意图证明下游保持了相同契约。
模型选择解决策略；Core 守住权限、预算、协议一致性和真实终态；Host 提供观察，不得把策略偏好变成新的授权或完成条件。

## 整改前已发现的边界问题（因果记录）

| 问题 | 当前代码与轨迹 | 影响 / 确认程度 | 目标契约 |
| --- | --- | --- | --- |
| 策略失败变成工具剥夺 | `model_planner.rs` 在 no-progress 时只保留 request_user_input；`submission.rs` 拒绝非结束/提问的下一轮 | music-harmony 重复 bash 后封存 Incomplete 已实证；为什么模型开始重复尚未证明 | 拒绝重复副作用不等于禁止所有替代方案；预算内允许有边界的不同策略恢复，耗尽后诚实 Incomplete |
| 恢复指令互相矛盾 | 重复 batch 结果写 `next_action=request_user_decision`；`turn.rs::model_instruction` 又明确 repetition 不是缺失用户决定 | 模型收到冲突建议，恢复阶段缺少可执行的自主出口 | 只有确实缺少用户选择才进入 NeedsInput；无进展是独立恢复状态，不制造问题 |
| 检查阶段以修改作为出口 | `turn.rs::admit_tool_execution_with_progress` 在阶段耗尽后要求现有证据作答或具体 mutation；planner 同时剔除调查工具 | 对只读分析、需要新观察才能纠错的任务不普适；不能据此断言每个失败都由该分支引起 | 进展由新观察、假设验证、资源版本和交付物变化度量，不能只有 mutation 才能恢复调查 |
| Host workflow 重复制定任务政策 | Core 已有 scope/tool/skill/execution 政策；`host_prompts.rs` CLI 另要求 SOP 激活，并禁止未明确请求的 scratch/environment 操作；`core_agent_runtime.rs` 再加 workspace workflow | 易把完成任务所需的普通局部实现步骤误作额外任务；与通用任务范围裁量相冲突 | Host 只给环境、能力、呈现事实；Core 一份行动边界，允许授权任务内必要且可逆的步骤，敏感/越界动作仍走权限 |
| Core cache shape 未贯穿 provider | `model_planner.rs` 每步追加动态 system 权限/恢复信息；`convert_messages_to_anthropic` 将所有 system 文本收集并 join 到顶层 | Anthropic 序列化会把动态信息移到历史之前；这是源码事实，不是本轮 LAN Chat 缓存低的已证根因 | 稳定政策与动态事实在最终 wire 上保持区分；按实际请求体比对 prefix，记录真实 usage，不用内部 hash 代替命中证明 |
| Transport 有第二套总预算 | Chat/Anthropic 之前在工具参数首片段后固定 45s 截断 | 已删除；两个 48s socket 回归通过。旧 freight 失败间隔吻合但旧 generic error 不足以严格归因 | Transport 管理连接/空闲，不偷偷缩短任务预算；完整事务才执行，内存/输出/取消各自保留明确边界 |
| 取消控制只覆盖部分入口 | Native Host 安装 SIGINT/SIGTERM；JSON one-shot 默认 control，终端 listener 在非 TTY 直接返回 | 静态入口缺口明确；前批 timeout 无 seal，仍需受控信号实验串联因果 | 所有进程入口接同一 Runtime control；Core 决定取消并封存；SIGKILL/崩溃保留 unknown，不伪造 Cancelled |

## 参考项目真正提供的启发

pi 的 loop 把工具错误作为 `isError` 结果回到同一上下文，模型继续选择下一步；工具变化有显式声明。它也拒绝执行 length 截断的工具参数，并不意味着可以去掉事务安全。
OpenCode 同样有三次重复检测，但把它作为单独 doom_loop 权限请求，不能笼统说其他项目没有限制，也不应直接复制为 Nola 的用户等待状态。
Claude 提示词资料强调按原始范围交付、常规判断自行处理、只有实质缺失选择才阻塞；其短 harness 说明没有要求模型操作一套通用阶段台账。

Nola 应保留持久事实、权限和诚实完成判定，削减的是竞争的策略控制器，而不是安全与落库边界。
缩短提示词本身不能修好工具被剥夺、动态消息被前移或取消入口未接线。

## 整改顺序与验收门槛

1. 固化实际 Step 输入快照：政策片段来源、最终工具 ID/schema digest、动态消息位置、provider body 形状、输出预算与错误类别。诊断不记录密钥；host catalog 大小不能冒充 wire 工具数量。
2. 合并恢复控制：明确正常执行 / 协议修正 / 无进展策略调整 / 等待用户 / 权限等待 / 预算耗尽。一次失败仅有一个恢复决定所有者；替代调用仍通过原权限核验，不给补救过程额外授权。
3. 精简 Host workflow 与模型台账负担，保留用户真实约束、必要工具语义和证据；按最终序列化检查不同 provider 的稳定前缀。
4. 统一信号、超时、取消、冷恢复；外部进程结束和业务终态分别留证。
5. 验收包含：只读调查在获得新证据时可继续；重复副作用不再执行但不同策略可恢复；错误不转换成虚假用户问题；截断无部分执行；真实信号到终态落库；压缩/恢复后上述状态不丢；多语言和自然追问不经关键词路由。

四题真实测试用于暴露集成问题，不向 prompt 或生产代码加入题名、参考解、特殊路径规则。模型理解错误与运行时阻断必须分开归因。采样变化可能改变结果，不能由一次更好的轨迹宣称架构整改完成。

## 附录：harness-v4 两题纵向审计与未完成契约

证据根目录为 `.artifacts/evals/terminal-bench-runs/lan-20261013-batch3-harness-v4/trials/2026-10-13__15-27-00/`。本附录来自已结束试跑的 `result.json`、artifact manifest、`agent/nola.ndjson.gz`、`agent/nola-evidence.json` 和评分输出；不是新测试结果。资源依赖重新准入及错误诊断方案仍未实现、未验收。

### Photonic：产物错误与模型请求失败是两个事实

`photonic-waveguide-routing__eViMpEZ` 的 `/app/routing_result_1.json` 收集成功。官方两项实际任务检查失败：多条路径穿越障碍、网间距不足 8µm；另外十二项通过是评分器自测，不是任务局部成功。评分为零。

七次工具调用构成完整轨迹：读取规格两次、写 `/app/scratch/validate.py`、写候选布局、执行检查发生 `KeyError: 0`、修正脚本访问 `waypoints`、再次执行检查 exit=0。自写检查器只打印 `VIOLATIONS`，没有通过非零退出码表达违规；成本循环仍将包装字典传给 `cost_net`。因此 **exit=0 只证明进程退出约定，不证明布局正确**。Core 当时显示 `ready_to_complete` 不能替代正式需求验收。

下一次模型请求开始后失败，CLI exit=1，终态 seal 的 `result.value.kind=failed`，保留七条已观察 effect refs，无 unknown refs。这里没有重复观察拒绝，不能归因恢复策略封路。

最后一轮 `model_generation_started` 为 08:03:32.602，`accepted` 为 08:03:32.689，`run.failed` 为 08:04:12.989：事件链间隔约40.4秒，不是直接测得的 HTTP 耗时。公开错误只有 `openai-compatible/qwen3.8-flash-next-iq3_s: The model request failed.`。保留证据里没有该轮原始错误、HTTP状态或网络阶段，不能据间隔断言超时、服务重启或服务端错误。

### FreeCAD：调查未转化为交付，真实超时取消已封存

`freecad-platform-drawing__V8y95NH` 两个交付物 `/app/build_part.py`、`/app/part.FCStd` 的收集状态均为 failed。图片读取产生真实 PNG 快照 artifact，已不是旧版“不支持读PNG”；artifact本身不能证明每次最终 provider body 都包含图片。

第一次请求约180.6秒后发生重试，之后才开始工具执行。44次工具请求包括1次图片读取、43次bash；其中34次exit=0、4次exit=1、5次被Core拒绝执行。主体是手写PNG解码、保存 `/tmp/img.npy`、重复像素线段扫描，没有创建建模脚本。末段四次 `repeated_observation` 和一次 `repeated_tool_batch` 要求更换策略，模型仍重复原扫描；不能把失败全部归因Core工具剥夺。

压缩从08:19:05.121到08:19:25.917，事件口径20.796秒。摘要保留脚本尚未创建，也保留了题面28800秒，而开发试跑supervisor只有600秒：这是预算上下文不一致，不是摘要谎报完成。supervisor最终exit124、forced_termination=false，Core生成 `cancelled` seal和 `execution-interrupt:1`，本次不是宿主崩溃或无终态。

评分环境另因pip无法卸载conda/distutils安装的VTK9.2.6而构建失败。该基础设施失败与模型未交付必须分别报告。

### 时间证据不能混用

| 试题 | supervisor monotonic | 同一supervisor wall | wall减monotonic |
| --- | ---: | ---: | ---: |
| photonic | 98.493s | 399.988s | +301.496s |
| FreeCAD | 603.345s | 1225.961s | +622.616s |

四题进程monotonic合计1616.149秒，wall合计3636.239秒；Harbor还包含准备、收集和评分。campaign显示的30/68分钟不能直接当LLM耗时。差值证明计时口径存在偏差，不足以判定具体休眠或时钟跳变原因。

### 资源依赖重新准入：已有能力与缺失输入

现有 `resource_evidence.rs::validation_snapshot` 对显式 `validation_targets` 前后取版本；`attach_validation_evidence` 在成功且版本稳定时生成 `PostconditionCheck`，predicate为 `process_exit_zero_with_stable_resource`。`turn.rs::reconcile_execution_receipt` 能用版本匹配消除对应义务。普通write/edit有after_version，但 `with_file_artifact` 的before_version通常为空。

缺失的是检查过程的依赖集合。photonic两次检查只声明布局JSON，中间修改的是检查脚本；当前checks无法证明脚本与检查的依赖关系。不可从命令字符串猜测，也不可把模型申报路径直接当Host已经观察的版本。当前成功mutation清空整个 `investigation_evidence` 的做法过宽；仅凭成功exit不能证明新证据。

建议的typed边界（设计，尚未实现）：

| 输入/输出类型 | 所有者与必要字段 | 契约 |
| --- | --- | --- |
| `ObservationScopeRequest` | 模型提出；resource refs、声明的依赖用途 | 是待核验声明，不是权限或证据；拒绝越权路径 |
| `ObservedResourceVersion` | Host产生；规范URI、版本方案、版本值、观察call id、观察阶段 | 文件内容/元数据、目录、环境、远端资源分别有类型；模型不能填写版本作为权威 |
| `ObservationWitness` | Core随结果提交；调用signature、结果digest、Host版本集合、scope completeness | 只有已提交Host观察可组成依赖前沿；opaque/不完整scope保持unknown |
| `ResourceEffect` | Host已有；URI、before/after version、operation | 缺before时仅可和已存同资源版本比较；无可比版本不构成变化证明 |
| `ReadmissionDecision` | Core；允许/拒绝、关联witness及effect refs | 仅相交依赖版本变化可解锁同调用；不补恢复预算、不产生验证通过信用 |

权限核验始终先于Host观察。外部修改需有界的新Host版本观察，不能令已拒调用永久没有刷新通道，也不能每轮全盘哈希。普通文件目前只有内容hash，执行权限变化不包含其中；需明确元数据版本，不能宣称已有内容hash覆盖chmod。显式依赖即使版本稳定，也只证明所声明资源，不能伪称捕获任意shell全部输入。

测试矩阵应包含：相关脚本变化允许重查；无关文件变化不解锁；同内容写入不解锁；无版本effect不解锁；模型伪造版本不采信；权限变化重新核验；失败检查不完成；外部变化通过新Host观察解锁；部分依赖保持unknown；压缩/重启保留witness与累计预算；exit0但仅打印违规不等于官方验收成功。

### 通用模型错误的信息丢失链

`llm_parts/streaming.rs::ProviderFailure::classify` 把原始错误映射为有限code/message，同时在内存保留 `diagnostic`。未识别的transport/parser错误落入 `provider_failed / The model request failed.`。`orchestration_execution.rs` 的最终错误分支把code、message、retryable、http_status交给 `HostResponse::Failed`；可选 `model_provider_failure` debug记录仅含code/status/retryable/provider/model，并未记录diagnostic或请求阶段。最终公开run trace只有泛化message。该题导出的agent目录只有事件与证据JSON，没有私有debug日志，故无法恢复原始错误。不能以源码可能生成diagnostic冒充本题已有诊断证据。

改进方案应保持用户提示简洁与密钥安全：在transport产生typed phase（connect/headers/read/decode/consumer）和错误类别、attempt id、请求关联id、monotonic elapsed、已接收字节/是否已提交输出、可选HTTP状态；沿既有HostError/模型attempt记录保留到持久化诊断投影。不得记录Authorization、带查询参数URL、完整请求/响应正文。没有已知phase或cause时显式unknown，不通过宽泛字符串猜测。测试应逐段注入断连、空闲超时、坏SSE、非2xx与consumer拒绝，检查同一安全错误码从provider到seal关联事实未丢失；这仍是待实施方案。
