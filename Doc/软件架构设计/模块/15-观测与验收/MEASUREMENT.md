> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 性能测量接口与基线采集契约

状态：最小 Rust 生产测量接口及调用点已实现，已有本地主链基线与真实 Mac 单请求采集试跑；完整阶段覆盖与性能预算仍未完成。细化 VERIFICATION P01–P08。单请求结果不代表性能达标。
性能记录是诊断，不是完成、授权、消息或 usage 账本的第二份真相。

真实断流复现并整改（2026-10-12，正常路径真实复验通过）：此前 provider_sampling 的
succeeded 只表示适配器返回 Ok，stream_incomplete 也会误记。现正常采样与请求
快照分叉共同调用 StreamedChoice.sampling_measurement_outcome：stop/end_turn/
stop_sequence/completed/tool_calls/tool_use/function_call 为 Succeeded，
protocol_error 为 Failed，content_filter/refusal 为 Rejected，其余含缺失/未知/
截断为 Incomplete。这里只分类采样诊断，不改变业务结果、不丢弃部分文本、不
发起重试；Succeeded 也不是任务成功或工具已执行。旧构建的计量不能据新规则
重新声称正确。模块回归 152 项通过；新构建真实故障样本
`/tmp/nola-mac-real-memory.ztgswO` 已验证 1025 字节断流采样为 incomplete、
后续完整流为 succeeded，最终 UI/seal 验收通过。压缩分叉的新真实样本见下节；
尚不能据单个样本声称全部计量路径覆盖。
HTTP headers succeeded 仍只表示收到响应头，不等于生成成功。原始故障样本
`/tmp/nola-mac-real-memory.xRgVcv` 保存断流和重试两次请求，详见 VERIFICATION。

## 验收用 provider trace 统计边界

### 真实压缩计量样本（2026-10-12）

隔离 Mac 样本 `/tmp/nola-mac-real-memory.SEJqYo` 实际触发 128-token 输出截断，
458 字节部分摘要经一次续写提交，原文召回 XCTest 1/1 通过。sampling 对截断记
incomplete，对完整续写记 succeeded。原 runner 因读旧 Trace 表 exit 1，改为读取
canonical run_events 后离线证据校验通过；完整门禁重新运行的结果单列，不回写
原失败记录。该样本不是 UI 帧延迟验收，详细流程见 VERIFICATION。

| 阶段 | Host 请求 elapsed_ms | cache read / input |
|---|---:|---:|
| 热分叉，被截断 | 2459 | 20315 / 20988 |
| 同一摘要续写，接受 | 4458 | 21115 / 21169 |
| 后续热分叉，接受 | 4807 | 21191 / 21893 |
| 冷增量摘要，接受 | 3121 | 0 / 4594 |

按 attempt id 取最后 revision，持久账本有 12 个 attempt，其中 4 个 compaction；
代理日志也有 12 个模型响应（其余 7 个非模型记录不计入）。这只是数量核对，
不是逐请求 identity 对账，也不能排除请求未落代理日志。账本可比较输入合计
218367、cache read 合计 176731；这是该隔离会话样本，不是产品整体缓存率。
不得以成功样本取代失败/缺测分母，也不据单样本制定 p95 或性能预算。

`scripts/provider_trace_metrics.py` 是回执生成器、中央判定器和 Mac runner 的共同
统计实现。输入为只读 JSONL 文件集合；同一路径重复引用只读一次。字段错误计入
malformedRows，无法读取/非法 UTF-8 计入 unreadableFiles，真实 provider 门禁拒绝
这些不完整证据，不把读取失败补成零请求的成功报告。

| 字段 | 口径 |
|---|---|
| observedModelRequests | 有效 response 记录中模型生成路由的数量，含 HTTP 失败/未取得状态 |
| successfulRequests | 有非空响应体的 HTTP 2xx，含模型发现等非生成请求 |
| successfulModelRequests | 上述 HTTP 成功中属于模型生成的部分，不代表完整生成或 Run 成功 |
| completedModelStreams | HTTP 成功、无转发错误/注入断流，并有当前 probe 可识别完整流终止证据 |
| successfulModelUpstreamHosts | 仅成功模型请求本身的上游，不能借用无关发现请求的远端地址 |

生成路由按结构化 upstreamPath 的路径末尾识别 chat/completions、responses、messages，
要求非空 model；模型发现、responses/compact 不纳入此“生成请求”计数。结构化 external
executor 保持要求 backend、sessionId 和 structuredEvents。路由不是用户自然语言分类。
当前完整流证据只覆盖 probe 已识别的 Chat/Responses 终止字段；未观测到不能推断完整。
这些是已记录响应的统计，不是请求发出前的清单：崩溃导致未落日志的请求、跨不同副本
文件的同一请求去重、所有协议的完整流判定仍未覆盖。不得拿它作为全提交分母、任务
成功率或缓存命中率；Run 完成仍由正式 canonical terminal/seal 证明。

## HTTP 等待限制与性能测量不是同一契约

真实模型验收必须使用正常运行的等待配置，不得因为运行于 XCTest 就继承 mock 的
5 秒流超时。Mock 的短等待只约束确定性故障测试；真实验收的整场 watchdog 可单独
有界，但不能冒充 provider 超时。真实输出截断注入必须标记请求额度的变化，并要求
上游自己返回截断原因，禁止伪造 SSE。缺失 usage 的物理尝试保留为计量未知，即使
后继重试完成任务，也不能在预算验收或缓存统计中将前一次当作零成本成功。

当前 Rust blocking transport 使用 reqwest 0.11。下表是实现中的保护配置，不是由
实测制定的性能 SLA，也不是模型允许生成的总时长。

| 边界 | 当前实现 | 禁止混淆 |
|---|---|---|
| 建立连接 | connect timeout 30 秒，独立配置于 builder | 不等于首 token 或整个响应期限 |
| HTTP 头及每次阻塞读取 | `NOLA_LLM_HTTP_TIMEOUT_SEC` 默认 180 秒 | 不把 Client blocking timeout 当作活跃流总时长 |
| 显式关闭 HTTP 等待限制 | none/off/false/0 → `.timeout(None)` | 省略 builder 调用仍会保留依赖默认 30 秒，不能当作关闭 |
| SSE 协议等待 | 各协议自己的首事件、空闲、工具片段窗口 | 收到 HTTP 头、注释心跳、首正文是不同测量终点 |
| 用户取消 | 控制接收方可结束等待 | 不等于底层阻塞线程和 socket 已停止 |

配置关闭只影响 HTTP 等待层，连接超时及协议窗口不随之关闭。当前 Chat/Anthropic
路径使用 first-event/tail/tool-call 窗口，Responses 路径使用单一 semantic idle 窗口，
仅非空正文/思考/工具参数增量及新验证工具身份推进时钟；注释、空行、空 delta、普通
生命周期事件不重置时钟。每次读取使用剩余时长，不能按收到行重新等待完整窗口。
这些窗口的配置尚未统一，不能宣称跨协议使用同一首 token 超时。Responses 超时有
文本则保留 `stream_incomplete`，无内容则返回超时，存在未提交工具调用则拒绝；
不把超时当作正常 EOF，也不由 transport 决定重跑模型。

事件与终局的区分依据 [OpenAI 官方流式说明](https://developers.openai.com/api/docs/guides/streaming-responses)
核对（2026-10-12）；哪些事件续期及超时数值属于 Nola 策略，不是官方规定的 SLA。

测试必须分开验证：已配置头等待超时、正文读取停顿报错、活跃响应总时长超过单次
读取限制仍成功、显式关闭时真实阻塞读取跨依赖默认 30 秒仍成功。最后一项使用本地
HTTP 服务，不是公网 provider 性能基线。关闭读取期限后，取消仍不能立即终止正在
阻塞的 reader；物理 abort 和线程回收是独立待验收项，不用前台已返回取消代替证明。

## 2026-10-12 真实 Mac 采集试跑

流所有权整改后新增一次当前构建样本（`/tmp/nola-mac-real-memory.RmfEKJ`）：
runner 1/1 通过、零跳过，1 个 answered seal 与界面记录一致，正文流门通过。
构建指纹 `20fd67ef02b1df0210a47480ca042215304a229d970775ea8aa0f1dc99ba2ce9`。
5 条 measurement 事件中的 adapter_first_text 相对 provider_sampling begin 为
2290.18 ms，sampling span 为 13983.45 ms，headers span 为 36.53 ms。
这是 adapter/transport 测量，不是用户发送到 UI 首帧。尚缺完整预登记提交分母、
多次重复与失败样本，不能与不同构建旧样本合并宣称 p95 或性能预算达标。

### 最新正文流正例（活动会话切换用例）

隔离 Test fresh build 的 `testRealStreamingSurvivesConversationSwitchAndKeepsLatestVisible`
在 `.LHbvB4`、`.HvCxlT` 两次真实执行均为 1/1 通过、零跳过；第一次 runner 被旧 Trace
非空要求拒绝，修正脚本后第二次完整 runner exit 0。第二次独立 provider 正文流门
通过：2352 个正文 delta，10648 字节，finishReasons=[stop]，DONE=1。不是工具参数
代替正文。Swift 实际检查活动流切换、草稿隔离、按钮状态、AX 多次变化、最终正文
编号/标记与底部重排。截图产物已保存，但不据自动化断言声明全部视觉设计达标。

| 原始产物目录（`/tmp/nola-mac-real-memory.` 前缀） | adapter 首正文 ms | sampling ms | 代理 headers ms |
|---|---:|---:|---:|
| LHbvB4 | 2239.634 | 29818.867917 | 44.1675 |
| HvCxlT | 2266.620125 | 30139.436333 | 30.293417 |

两次实际 App MacOS artifact-set digest 均为
`f6a13e95a12f10bdd245c6bd22535c131e7c868f7d15bf69264b11b6bf12245d`；每目录保存
`measurement/build.json`、`measurement/events.jsonl`、`runtime/provider.ndjson`、
`runtime/mac-product-ui-transcript.json` 和 `result.xcresult`。report_measurement_events
均识别 1 个成功 sampling/headers、1 个 adapter_first_text，无首 reasoning 样本。
2 个样本不能定 p95 或预算；headers 包含本地代理，不是远端网络首字节。仍缺预登记
实验清单、完整命令回执分母、UI 首帧 signpost 关联及正式 usage 缓存统计。以下较早
“正文流正例尚缺”的叙述是历史检查点，现只由本段限定范围的正例替代，不扩张验收。

当前真实 UI 门禁已切换为正式终态读回：Swift 对每个 assistant journal identity 调用
`canonical_run_terminal`，将精确 conversation/run/assistant、result、result_digest、
completed_at 导出到 transcript；只读 Python 比较器在同一 SQLite 快照核对 seal，
要求当前 `sealed` 和 answered/action_completed，拒绝缺失、重复、错配及该会话漏记
的 conversation Run。生产 Rust 继续拥有 manifest/result 的验证权，Python 不重写
其验签/摘要算法。旧 `runs.status='completed'`、旧 Parts 和 Trace 不再决定通过。
无工具的正文回答不要求 tool.completed；文件存在也不再被报告成“Markdown 已渲染”
或“流式前缀稳定”。这些 UI 行为须由具体 XCTest/渲染证据证明，不能由通用 runner
虚构。比较器测试只证明比对规则，不代表真实 Mac 场景已通过。

- 采集环境、报告及回执导出 Python 回归：31 项通过；runner shell 语法检查通过。
- 真实 Mac XCTest：1 通过、0 跳过、0 失败，复用隔离 Test 构建，没有替换用户 Debug App。
- 产物：`/tmp/nola-mac-real-memory.BWVqNX/measurement/{build.json,events.jsonl}`；
  XCTest 结果同目录 `result.xcresult`。这是临时原始证据，不是持久基线归档。
- artifact-set digest：`785ef8c57f1ef5dbd2ff9e07a1e65e2aa27f60cf5bbcdd22d7b0f0aa80d44037`。
  范围仅 App `Contents/MacOS` 实际文件，不覆盖所有外部依赖。
- 观察到 1 个成功 provider sampling，耗时 3688.691 ms；1 个成功 HTTP headers span，
  耗时 37.532 ms。请求经本地测试代理，headers 不能解释成远端模型网络首字节。
- 最终回答通过内容断言，但 provider trace 显示实际只返回 `requirements_update` 工具调用，
  `responseTextDeltaCount=0`、`responseReasoningDeltaCount=0`。该工具的已验证 `answer`
  可合法生成 canonical 正文；它不是 provider 正文 delta。因而没有 adapter first text/reasoning
  point 符合实际路径，不能断言采集丢失，也不能以 sampling 总耗时替代首正文延迟。
- 未提供发送前实验 manifest 或 canonical receipt snapshot；完整提交分母、缓存命中率、
  UI 首帧、跨进程端到端耗时均未验收。下一步是取得真正正文流样本和真实 receipt 身份预登记，
  不能由成功回执反推全部请求。缓存指标必须读正式 usage 账本。
- 首次试跑在发模型请求前发现 XCTest `TestHostPath` 为 `.app` 目录，采集 helper 原先只
  接受可执行路径。现已支持两种宿主路径并加入回归；失败产物保留于
  `/tmp/nola-mac-real-memory.Cg60eA`，不得计入成功采样分母。

### 最终回答与正文流必须分别验收

真实 Mac 用例已改名为 `testRealConfiguredModelProducesCanonicalAnswerThroughMacAgentHostWhenExplicitlyEnabled`；
其通过仅证明 canonical answer，不证明 provider text stream。工具携带回答是正式契约，
不可为了指标好看将工具参数伪造成 TextDelta，也不可删除正常工具来改变生产链路。
`NOLA_MAC_REAL_REQUIRE_TEXT_STREAM=1` 为 runner 的独立显式验收门：必须在实际 provider trace
观察到成功的 streaming response 且正文 delta 数量和字节均大于零；仅工具、仅思考、非流式、
失败或注入中断不满足此门。即使通过，也不证明 UI 已绘制或端到端首帧预算达标。

该门的真实反例验收：`/tmp/nola-mac-real-memory.31fuqn`，重新 build-for-testing 后
canonical-answer XCTest 1/1 通过，独立正文流验收拒绝，runner exit 1；模型再次只输出工具。
因此这个 run 不能登记为正文流通过。Python 回归为测量相关 31 项加正文流门 3 项，全部通过。
这里验证了旧假阳性会被拦截；真正正文流的正例和首输出性能数据仍待采集。

正文流验收的完成边界：HTTP 200 不是流完成。Chat Completions 必须具有正常
`finishReasons`（stop/tool_calls/function_call）与 `[DONE]`；Responses 必须观察到
`response.completed`。length/incomplete、缺终止标记或 oversized line 造成的观测缺损
不满足此门。完整 `message` 的正文与思考可统计字节，但不得统计为 delta。
2026-10-12 补充验证：provider probe/stream acceptance unittest 12 项通过，
`scripts/e2e/test_provider_probe.py` pytest 6 项通过。该门只证明至少一条完整正文流
被观察到，不证明所有请求成功；重试、失败前驱和完整提交分母仍由实验清单逐项核验。

## 已有计时点的实际含义

| 来源 | 实际观测点 | 不能据此声称 |
|---|---|---|
| cli/main_parts/bootstrap.rs | 参数、Host request、runtime config；session lock recovery | 用户发送到首 token |
| cli/core_agent_runtime.rs | Store、code intelligence、MCP、instructions、host acquire | 每个阶段都已优化；它们不覆盖首帧 |
| cli/main_parts/commands.rs | data source prompt preparation | 完整上下文组装/压缩耗时 |
| cli/agent_host_driver_parts/orchestration_execution.rs | 每个 physical_round 从 model_started_at 到第一个非空 TextDelta | 网络 first byte；该间隔包含 stream adapter 内部工作 |
| 同上 first_visible_text | presentation gate 首次放行、尚未调用完下游 emit | 用户真正看到的正文；尚未覆盖提交、订阅、布局、绘制 |
| core/nola-store/src/commit_diagnostics.rs | 显式同线程 capture 下的 disclosure flush、SQL commit、archive；最多 128 条 | 跨线程完整请求轨迹；默认不采集 |
| NolaMac/Services/MacCanonicalConversationRepository.swift / MacWorkspaceModel.swift | 可选 signpost 的 window apply / workspace publication proxy，generation/cursor/revision 关联 | 实际帧延迟或流式首帧；coalescing 100ms 仍只是上限参数，signpost 尚未真实采集 |

现在不能把两个名称带 first/visible 的 Host 日志相减当 Mac 绘制开销，也不能把多次 physical_round
合并成一次 provider TTFT。读取 debug 日志可能缺样本，缺失不补 0。

## PERF-EVENT：统一测量事件

以下为完整目标逻辑 schema；下节单独列出实际 V1 子集，不把未实现字段算作已有覆盖。持久业务 DTO 不为采样而增加可改变语义的状态。

```text
MeasurementEvent {
  schema_version,
  observation_id, experiment_id?, build_digest, process_instance_id,
  trace_id, span_id, parent_span_id?, clock_domain_id,
  monotonic_ns, stage, event: Begin | End | Point,
  outcome?: Succeeded | Rejected | Failed | Cancelled | Incomplete,
  correlation: { command_id?, conversation_id?, run_id?, effect_id?,
                 attempt_id?, physical_request_id?, projection_revision? },
  dimensions: { platform, profile, request_purpose?, protocol?,
                cold_state?, reasoning_tier?, tool_count?, history_size_bucket? },
  counters?: { input_bytes?, output_bytes?, records_read?, records_written?,
               retry_index?, queue_depth? }
}
```

### 已落地的 Rust V1 子集

`core/nola-measurement` 是独立诊断运行库，不依赖 Domain、Kernel 或 Store，也不新增业务数据库表。
通过 `NOLA_MEASUREMENT_FILE` 显式启用 JSONL 输出；未配置时不创建线程/队列、不取计时时钟、不格式化事件。
后台线程初始化失败则禁用诊断，不影响业务。首次打开文件、序列化和刷盘全部在专用线程进行；
文件打开或写入失败会断开接收端。`enabled()` 仅表示已配置采集器，不保证文件打开成功或观测已落盘。
业务线程使用容量 1024 的
`try_send`，满队列/断开的输出端只累加 dropped。Unix 新文件权限 0600。进程退出不强制等待诊断刷盘，
故末尾事件可以缺失；`dropped_total` 为成功送出的事件携带的累计快照，不保证最后一次丢弃一定有后续事件报告。

事件字段为 `schema_version=1`、可选 `build_digest`、`process_id`、每进程随机 `clock_domain`、
`observation_id`、`span_id`、十进制字符串 `monotonic_ns/elapsed_ns`、`stage/event/outcome`、`context`、`dropped_total`。
构建摘要仅从 `NOLA_MEASUREMENT_BUILD_DIGEST` 读取合法 64 位十六进制值；缺失保留 null，不能用于冒充已固定构建的基线。
`context` 含 SHA-256 后的 session/effect/request/attempt/command/run 身份、typed command_kind、parent_span 和进程内 physical_request 序号。
不记录 URL、正文、工具参数、路径、密钥、provider 错误原文或任意用户维度。

实际调用点：

- Application `execute_received_command`：`command_execution` 覆盖每次调用（包括回执重放、采样前拒绝或失败）；
  `command_receipt_committed` 仅在 SQL COMMIT 成功且非 replay 后发 point。point 成功指 **命令回执 committed**，
  不证明新 Run、模型请求或任务完成；rejected/conflict 对应 rejected，恢复待定对应 incomplete。
  SQL 已提交但随后 Runtime adoption 失败时仍保留 committed point，外层调用 span 则失败，不回写业务事实。
- `llm_parts/transport.rs::send_request_with_cancellation`：每次真正发送（含 DNS、非流式 fallback）独立 physical_request；
  `http_headers` 只测到返回 headers/错误/取消，不是完整流、不声称网络 first byte 或请求已在服务器停止。
- 普通模型采样与 `request_snapshot.rs::CompactionFork::stream`：`provider_sampling` 完整适配器调用、首次非空
  `adapter_first_text/adapter_first_reasoning`。这些是适配器事件，不是 UI 首帧；热分叉和截断续写共用这条路径。
  普通流的实际 Host attempt 在 SamplingStarted 准入 barrier 后传回 producer 测量上下文，不能把 outer physical_round 当物理请求数。
- Host `execute_tool_with_sop`、`execute_compaction`：区分成功、失败、明确取消、emit 被拒和未闭合；
  工具 Deferred 保持 incomplete，不把已接纳后台工作算执行成功。Drop/展开失败默认 incomplete，不推断取消。
- Store 外层 UoW/独立 repository COMMIT：`store_commit`；嵌套 savepoint 不发此阶段。
  每个 UoW 最多暂存 128 条原 Host effect/request 关联，仅真实 COMMIT 后发 `store_committed_host_response`，失败明确 failed，
  rollback/poison 清空暂存。该事件是关联 point，不给它虚构 0ns commit 延迟。
  可选 attempt 身份仅对不超过 64KiB 的原信封作有界身份解码；更大记录仍保留 effect/request，并计 dropped，不能声称精确 attempt 覆盖完整。

跨进程依靠 hashed effect/request/attempt 关联，禁止直接相减不同 clock_domain 的单调时间。
Store 的 point 与 commit span 以 parent_span 关联；HTTP physical_request 与 provider_sampling 也以 parent_span 关联。
原 `commit_diagnostics` capture 继续提供 flush/sql/archive 更细的局部观察，不复制业务结果。
配置必须在进程首次诊断调用前设置；运行中修改环境变量不会重新初始化采集器。

`scripts/report_measurement_events.py BUILD_SHA256 FILE [FILE ...]` 提供 V1 JSONL 离线汇总
（实现与测试源码已添加，尚未运行）。只读原始文件、向 stdout 输出脱敏 JSON 与输入内容摘要；
不导出路径或事件中的自由文本。不同/缺失构建、格式错误与重复 observation 单独计数。
相同 process/clock/observation ID 的完全相同记录仅去重；内容不同时记为
`conflicting_observation`，所有关联 span（包括首输出 point 的 sampling parent）
退出耗时分布，不以输入文件顺序选胜者。冲突 point 不计入有效 point 数。
冲突、缺失和普通执行失败是不同诊断分类；相关逆序及跨 span 反例仅已补源码，尚未运行。
span 必须在同一 process/clock_domain 下唯一 Begin/End 配对，时间倒序或重复端点不产生耗时样本。
同一 span 若声称属于多个 stage，在每个被声明的 stage 下各记一次无效关联，
不按第一个文件选择归属；因此各 stage 的 observed_spans 不能直接相加当全局请求数。
缺端点记 unpaired，失败/取消/未闭合按 outcome 分组，绝不混进成功分位数。
不同 clock 的 dropped 累计各取最大后求和，仅为已观测下界；不能声称采集完整。
普通 point 只计数量，不推算延迟、请求数或缓存率。首文本/思考 point 有独立的受限派生规则：
仅当同一 process/clock、parent_span 指向唯一 provider_sampling Begin，且 session/effect/request/attempt
全部一致，才计算 Begin 到 point 的单调时间差。重复首输出、越界时间、错误来源或跨时钟均不选最快值。
报告 `adapter_first_output` 的分母只是已观察 sampling span；缺输出单列，终点缺失记 censored，
失败/取消请求的首输出与成功请求分别统计。这个指标终点在 adapter 回调、下游准入之前，
不是网络 TTFT 或用户实际看到的首帧，也不证明一次 sampling 只有一次 HTTP send。
上述派生规则及反例仅新增源码，尚未运行。局部 span 报告没有准入分母，不能给出全请求成功率，
也不能用其代替带实验清单、全部准入样本与真实 UI 帧证据的基线。

未完成：跨层完整 command/run/projection 身份接线、全部启动/上下文/排队阶段、HTTP body 网络首字节、客户端订阅与实际绘制帧、
完整实验报告聚合以及全部性能基线。当前实现不能据此声称首帧/压缩性能达标。
本轮只补实现、测试源码与静态审计；统一验收阶段才运行测试及基线，不在此处提供伪造结果。

### Application 回执与实验分母核对（2026-10-12，源码已接线，待统一验收）

唯一当前 schema 仍为 1。command 是 `SHA256("nola.command.v1\0" || len64be(client UTF8) || client UTF8 || len64be(request UTF8) || request UTF8)`，
与 Host request 身份独立，不把两类 request 混接。run/session 保持 SHA256 原身份。
command_kind 来自 ApplicationCommand 枚举，不使用调用方 command_type 文本或用户关键词。
新 Run 只从 Begin/Task/Automation/Promote 的已知 AcceptedAssistantTurn 返回形状取身份；active submission 取 typed input.run_id。
idle/other 没有可靠 Run 身份则保持缺失，不通过扫描任意 JSON 补猜。

`scripts/export_measurement_receipts.py DATABASE EXPERIMENT_INPUT_JSON` 在指定**隔离实验库**打开只读连接，
一个读快照内按全部样本的 client_id/request_id 查询 canonical command_receipts，仅向 stdout 导出 hash 身份和状态。
输入样本需同时给出 command hash 以校验 tuple，重复身份拒绝；没有回执的样本导出 absent，accepted 未闭合导出 recovery_required。
不导出 command_json/result_json/错误/路径，不重新累计 usage。数据库不可读或 schema 不符直接失败，不能把查询错误当 absent。
实验 harness 必须从发送前登记全部样本，而不是从成功回执反向生成列表。

报告入口：`report_measurement_events.py BUILD FILE... --experiment MANIFEST --receipts SNAPSHOT`。
MANIFEST 为 `{schema_version:1, build_digest, canonical_receipts_sha256, samples:[{command, result}]}`；
result 是 succeeded/failed/cancelled/timeout/pending，指实验观测结果，不由诊断 span 推断。
SNAPSHOT 使用上述只读导出内容 `{schema_version:1, build_digest, receipts:[{command,status,run?,session?,run_binding?}], attempts:[...]}`。
manifest 必须保存实际 snapshot 字节的 SHA256；构建、schema、摘要不符拒绝汇总。完整机器/配置/场景清单仍按下节要求留存。
report 保留全部样本数、canonical committed/rejected/conflict/absent/unknown/缺快照项、实验失败/取消/超时、
诊断缺失/歧义/身份或结果冲突，重复或相互矛盾的 receipt point 不选一个当真。canonical committed 命令数不等于 Run 成功率。
清单以外的回执/事件单列，不改变分母。事件丢失不反向修改 canonical 状态；没有采样的样本仍在实验总体里。
适配器首输出报告仍单独保留 observed_sampling 分母；以下 canonical join 另给出全部样本口径，不将两个分母混用。
新增 tuple hash、replay 分类、拒绝/冲突、只读快照、缺样本、重复/冲突和不匹配测试源码；尚未执行。

canonical join 只沿已提交结构：已知开 Run 的回执 `result.begin.run_id` 或 active submission 的
`input.run_id` → `runs` 校验 conversation owner → `run_attempts` → `effect_outbox` + delivery。
`run_attempt_deliveries` 的同 request 重试映射会被后继 Attempt 替换，因此还 union 已提交
`host_event_inbox` envelope 的 attempt_id/request_id，且同时匹配 run/effect；不能漏掉前驱 Attempt。
导出仅保留哈希身份、state、predecessor、Run conversation/maintenance 和 effect model_stream/compact 分类，
不导出 usage 或 payload。没有 delivery 的已准备 Attempt 仍保留（request=null），不能删去当没有尝试。

`command_output` 用 session/effect/request/attempt 四元组精确连接 sampling，不按同会话或时间邻近猜测。
canonical Attempt 数、重试、压缩、maintenance 分列；压缩摘要绝不计作用户答案首 token。
全部 manifest 样本逐项归为未 committed、无 Run 关系、existing/shared Run 不归属、无输出、
缺起点、跨时钟、身份冲突或同钟输出。仅唯一 opened_run 回执且 receipt point 精确指向本次
command_execution span 时，可计算 **Application 调用开始→最早已观察适配器输出代理**；相同时钟只取已观察点最小值，
不把它称为 Run 实际首次输出。前驱 Attempt 的采样/首输出事件可能丢失；报告并列 canonical conversation Attempt 数、
已观察采样 Attempt 数、已观察输出 Attempt 数，以及缺采样/缺输出证明的样本数。即使后继 retry 有输出，
前驱未观测仍显式标覆盖不完整；正常无输出与丢失输出点无法区分，不猜零输出。
延迟字段明确命名 observed_onset_by_sample_result，禁止将代理当完整 TTFT。要求 command begin≤receipt committed≤observed output，
若 command end 存在还要求 receipt 在 span 内；逆序/损坏点不产出时延。
重放调用不会另发 receipt point，不能把新调用起点套到旧答案上。同 Run 的 follow-up 不拥有整个历史 Attempt，
其单条输入归因仍标不适用而保留总体样本。不同 clock 只关联身份，不相减；这是端到端链路的本地子段，
不是 Mac 点击发送→真实可见帧。新增两 Run 同会话、重试 delivery 旧映射、跨钟、compaction/maintenance
不抢用户输出、prepared 未采样与准确身份反例测试源码；全部留最终统一验收。

- Begin/End 同 span、同 clock_domain，End 不早于 Begin；缺配对标记 missing/censored，不能补计时。
- process_instance_id 每次进程启动不同；单调时间只在已验证相同 clock domain 内计算差值。
  跨进程传 trace/span 关联而非假定各进程零点相同；跨设备分别报告阶段及往返观测。
- physical_request_id 对实际 provider 调用唯一，包括 fallback/续写/重试，不能用一个 run_id 覆盖全部尝试。
- 查询/流片段重放不重复创建同一次测量。usage 统计仍读取 canonical 请求账本，不从日志重新累计。
- 凭据、请求正文、文件原文、完整绝对路径不进入 dimensions。业务 ID 仅在本地受控记录关联；
  汇总标签限于低基数枚举，不能把每个 conversation_id 当成 metrics label。
- 采样/写诊断失败不阻塞业务提交，不把日志 flush 放在首 token 的同步路径；有界队列满时计 dropped_count。
  关键验收实验须核对掉样，掉样不能隐瞒为性能变好。保存策略和容量必须在运行报告中声明。

## 阶段定义与终点

```text
UI submit → local pending persisted → Application admission committed → ACK observed
          → configuration/context ready → effect queued → effect start ACK
          → provider attempt begin → first reasoning / first text → provider terminal
          → canonical part committed → projection applied → frame presented
```

不是所有阶段串行；父子 span 保留并行关系，禁止把重叠耗时相加成为总耗时。
P01 从 UI submit 到 ACK observed，另报客户端准备与服务事务；P02 用相同 submit 起点分别报
首反馈、首 reasoning 首帧、首正文首帧。spinner 可以是首反馈，但不能计作文本 token。
工具 P03 把授权人工等待拆开；模型请求等待授权不应存在新的 physical_request。
压缩 P04 覆盖所有请求与最终 context commit；失败/取消也保留耗时和费用，不能只统计成功分片。
P06 的 frame presented 必须由 renderer/平台观测证明；队列入队、SwiftUI body 求值、stdout.flush
都不是通用首帧证据。无法观测帧时写明 terminal-write/UI-apply proxy，不能冒充首帧。

## 采集、预算与门禁

实验清单包含场景版本、隔离数据库种子、输入固定摘要、端点/模型版本、工具 catalog digest、
有效配置摘要、产物 digest、机器/OS、冷热定义、并发度。secret 不写入清单。
服务冷启动、连接冷启动、provider cache 冷启动必须分别标记；不能声称重开 UI 就清空服务端缓存。

每个条件至少 30 个样本，保留 sample index、结果、原始事件引用与测量完整率。
汇总同时给成功率、失败/超时/取消数、median/p95/max 和计量覆盖率；分母是所有准入样本。
没有成功首 token 的请求不悄悄从总体体验报告剔除；单独报告未到首 token 比例和等待至终止时间。
超时只代表下界观测，不能作为“恰好超时阈值”的成功耗时混进分位数。

预算决策表需填：baseline 数据引用、用户体验目标、可控阶段预算、provider 外部耗时、
可接受失败率、硬性正确性约束、超限后动作、批准日期与适用环境。未填写的项不是达标。
纯本地阶段可做固定 fixture 回归；真实模型/网络应并列对照，不以单次更快宣布改进。
观测开销用同 fixture 开关测量对照，日志采样方式变更必须重建可比性。

运行报告持久保存脱敏原始样本及 manifest 校验摘要，不能只链接会被清理的 /tmp。
原始数据缺失或 build/config 不匹配时判 insufficient evidence，不沿用旧版本通过结论。
[本地主链初始基线](../../../历史任务/实施与审计/PERFORMANCE_BASELINE.md) 已按新库/复用库各 30 样本采集 Application/SQLite 子段；
其固定 Host 输入与连接重开不能代替 provider、真实 UI 和进程恢复测量。
统一事件实现、完整 P01–P08 基线与性能预算仍未完成，对应范围见 VERIFICATION。

## Mac projection apply proxy（2026-10-12 源码，尚未验收）

`MacProjectionApplyMeasurement` 由启动前 `NOLA_MAC_PROJECTION_MEASUREMENT=1` 显式开启，
使用 `com.nola.mac / ProjectionApplyProxy` 的 OS signpost event，不创建诊断文件、
业务数据库、全局会话缓存或无界任务队列。事件仅包含固定枚举和标量；会话使用与 Rust
一致的 raw UTF-8 ID SHA-256，不输出原 ID、正文、路径、参数或错误详情。构建摘要沿用
`NOLA_MEASUREMENT_BUILD_DIGEST`，缺失写 missing，不据此接纳固定构建基线。

实际两个应用层终点及其范围：

- `MacConversationWindowModel.refreshIfChanged/load`：window/messages 主线程赋值后的
  `window_refresh_proxy` / `window_load_proxy`。刷新区间包括订阅等待、coalescing、读取及
  materialization；load 起点在 load task 建立/复用后、等待结果之前，不代表完整磁盘读取
  起点。刷新无新内容记 unchanged，过期 generation 记 discarded，失败/取消单列。
- 四个 canonical-window 调用点显式把 generation/eventSequence/projectionRevision 传给
  `MacWorkspaceModel.publishCanonicalMessages`，在赋值 `messagesByConversation` 后记录
  `workspace_publication_proxy`。相同消息未重发布记 unchanged。仅当前选中且 presentation
  attached 时 selected=true；后台会话更新不是选中页面更新。缺少来源 cursor 的其他调用
  不借用缓存窗口补造关联。Canvas 实际读取的是 `model.selectedMessages`；事件不在其
  body、onAppear 或布局中伪造 frame presented。

每对 begin/end 使用唯一 observation、进程 ID、进程随机 clock domain 和十进制 uptime
monotonic_ns；end 带 elapsed_ns、原 generation、应用 cursor/revision、结果及 selected。
只可对同进程/同 clock/同 observation 计算时间；会话 hash 只作跨 Rust/Mac 身份关联，
禁止相减不同 clock 的单调值。generation 过期不携带更新后窗口的 cursor；错误/取消也不
把旧窗口 cursor 当成本次应用证据。该格式是 OS signpost proxy，不是 Rust JSONL V1，
现有 `report_measurement_events.py` 不消费它。

最终需要 Instruments Points of Interest 与真实窗口帧采样同步记录这些 signpost；外部
采样再使用本进程时间轨迹、会话 hash/cursor/generation 对齐候选应用区间，实际可见帧
必须独立确认，不能把 end 当首帧。OSLog 可能丢样且这里没有完整率分母/掉样计数，缺点或
缺端点必须报告 missing/censored，不能宣称日志完整。窗口被遮挡、跨会话、快速切换、
长流式输出、真实帧跟随与开启/关闭诊断的开销对照均留待最终真实验收。
新增 XCTest 源码只验证 hash/字段、单调逆序拒绝、stale 过滤与非可见声明；尚未执行，
不代表 Swift 编译、signpost 可采集或真实绘制延迟已通过。

### Real Mac XCTest opt-in collection

`NOLA_MAC_REAL_MEASUREMENT=1 scripts/run_real_mac_xctests.sh` enables existing
Rust measurements through explicit XCTest environment injection. Select the
existing real text test with `NOLA_MAC_REAL_ONLY_TESTING`; this option does not
declare tool/model capabilities. The runner retains its isolated artifacts:
`measurement/events.jsonl` (0600) and `measurement/build.json`. Inherited
measurement paths/digests are not used, and user configuration is not modified.

The build digest hashes a sorted manifest of actual files in the selected
XCTest host app's `Contents/MacOS` after building, including embedded Agent Host,
service and debug dylib when present. It is an **experiment artifact-set label**,
not a claim that each emitting process has that binary hash or that frameworks,
OS libraries and every loaded image have been captured. Missing embedded Host
fails collection setup. No Git revision substitutes for artifact hashes.

`MacAppEnvironment.processEnvironment` already preserves these two variables;
no production environment whitelist changes are needed. The existing event
reporter remains the consumer. This hook alone does not provide a preregistered
command sample denominator, canonical receipt export, full cache usage totals,
or UI frame timestamps. Missing events are not zero latency. New helper tests
and a measurement-enabled real run remain pending unified verification.
