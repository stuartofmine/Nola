> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# 本地 Application / SQLite 实测基线

## 2026-10-13 真实任务补充观测（非 SLO）

原始隔离运行路径及数值见 [真实任务样本](../验收证据/real-task-20261013.json)。
按正式 Run 的时间边界筛选 provider 请求，排除验收器另起的取消探针；失败样本保留，
未封存任务没有完成耗时，不填零。缓存率为 token 加权比率，不平均各请求百分比。

CSV 编程任务 fZy6qS：Run 85.263s，7 次请求累计 81.351s，占 95.4%；
首轮请求 58.605s，返回 47,100 字节思考内容、11,971 output tokens。
这不是 TTFT，也不能据此归因于客户端初始化。其余 3.912s 包括工具、权限测试决议、
IPC、持久化、调度，尚无细分，因此不能全部记成工具开销。
脚本任务 ZiqAyv：Run 10.968s，4 次请求累计 8.939s，整体输入缓存率 61.59%。
四份样本任务/构建不同，不据此声称性能提升幅度或制定 p95 预算。

下一项主线核查是 **实际模型请求参数与思考生成量**：这些验收通过显式 fixture 将
reasoning capability 设为 false，实际线上请求没有 reasoning control，却仍收到思考。
“不声明支持控制”不等于“关闭服务端思考”；后续必须使用端点声明支持的参数做真实
对照，不能通过删除能力标记假装验证了简洁档。不得将缺失的 TTFT/UI 首帧数据补零。

状态：已执行的**本地主链阶段基线**，不是完整 P01–P08 验收，不是生产 SLO。
日期：2026-10-11。口径遵循 [MEASUREMENT](../../软件架构设计/模块/15-观测与验收/MEASUREMENT.md)，需求见 [VERIFICATION](VERIFICATION.md)。

## 实验身份与边界

实现：[独立 Rust integration harness](../../../cli/tests/local_pipeline_baseline.rs)。
同一个测试进程，顺序执行两种条件，各 30 个不同会话、30 个不同 Run：

- `fresh_database`：每样本创建独立空 SQLite 库，单独计 schema/open，再计准入。
- `reused_database`：同一个已打开 Application/SQLite 连接连续加入 30 个会话；包含数据库增长，
  不是 30 次对同一个消息的缓存读取。初始化空库的时间不计入此条件。

**这两个条件不称为整机/服务冷启动和热启动**：没有清 OS page cache，没有独立重启进程，
  也没有重置 provider cache。恢复阶段另开第二个真实连接，第一连接仍存活；因此只能叫连接重开。

实际经过 `execute_received_command`、Application Core reducer、真实 SQLite receipt/transition/
checkpoint/journal 提交；不是 mock repository。Host 的 ExecutionStarted、ModelAttemptStarted、
1 KiB TextDelta 是固定注入包，不运行外部模型、命令或工具。每样本验证非零 journal frontier，
第二连接读到的 frontier 与先前 ACK 后 frontier 完全一致，再加载 session projection，
断言会话身份和指定 Part 的完整 1024 字节正文均精确相等。
该验证不等于 SIGKILL、断电恢复、任务完成、终态封存或副作用重放验证。

机器：Mac17,3 / arm64 / 16 GiB / 10 logical CPUs；macOS 27.0.1 (26A434)，Rust 1.99.0。
构建：Cargo test unoptimized + debuginfo，mbx，16 MiB 测试线程栈；隔离 NOLA_HOME。
未读取生产配置、凭据、会话或数据库。未调用网络。测试临时数据由 tempfile 生命周期清理。
共享开发机没有做 CPU/磁盘静默控制；本轮其他 agent 同时运行 Rust 构建，存在明确的 CPU/磁盘竞争，
本批是**带竞争的初始样本**。冷/热固定顺序可能受顺序和后台负载影响，不据此断言某条件更快。

## 结果与测量完整性

复测命令 exit 0；1 个显式 opt-in integration test 通过，实验本体 12.91s，不含构建/锁等待 57.81s。
60/60 会话样本成功，510/510 预期阶段样本齐全，失败/超时/取消/缺失均为 0。
所有时间使用同进程 `Instant`。记录开销（JSON push、输出文件）位于 stage 计时终点之后。
没有完成测量开关 A/B，因此不声称已量化 observer overhead。

单位 ms，分位数仅对该条件 30 个成功样本采用 nearest rank；p95 为第 29 个排序值。
当前没有失败样本，但将来失败必须保留于原始数据并单列，不能填 0 或删出分母。

| 阶段 | 新库 median / p95 / max | 复用库 median / p95 / max |
|---|---|---|
| 创建空库及 schema/open | 14.959 / 18.901 / 19.007 | 不适用 |
| 准入回执 | 62.945 / 71.027 / 79.001 | 56.204 / 69.513 / 70.954 |
| claim 回执 | 1.344 / 1.717 / 1.718 | 1.362 / 3.078 / 3.775 |
| ExecutionStarted 回执 | 43.385 / 48.700 / 50.990 | 45.182 / 50.497 / 92.167 |
| ModelAttemptStarted 回执 | 44.341 / 50.434 / 50.530 | 45.313 / 58.328 / 75.026 |
| 首个 1 KiB TextDelta 回执 | 47.104 / 52.259 / 53.104 | 48.074 / 52.334 / 55.505 |
| 第二连接打开 | 1.990 / 2.693 / 2.891 | 2.658 / 3.276 / 5.379 |
| 第二连接读取 journal head | 0.084 / 0.111 / 0.123 | 0.092 / 0.112 / 0.148 |
| 第二连接加载 session projection | 1.808 / 2.516 / 2.989 | 1.898 / 2.032 / 2.134 |

回执计时从进入 helper 到收到 Committed 结果，包含 command JSON 序列化、准入校验、事务和
返回结果，不含 UI 输入处理、IPC、客户端通知/绘制。不能称为“用户点击到 ACK”的完整 P01。
claim 只测 claim 事务，不测排队等待、权限人工等待或工具执行；仅覆盖 P03 的本地协议开销子段。
delta 回执不是 P06 frame presented；恢复连接不是 P07 restart→interactive。

观察：即使无 provider，debug 下首次正文及两个开始事件回执也需约 40–56 ms，值得进一步分解 reducer、
checkpoint 序列化和 SQLite 提交成本。**本数据不能判定慢在 fsync、Core 或 UI**，也不能外推
真实分片频率的吞吐/帧率。下一步同 fixture 测 release 与不同历史/输出尺寸，再做内部阶段 profiling；
不可用跳过 durable commit、漏 journal、吞掉失败换低耗时。

## 原始证据与重复执行

本表仅使用复测证据：[`evidence/local-pipeline-20261011-v2/`](../验收证据/local-pipeline-20261011-v2/manifest.json)：
[510 个原始样本](../验收证据/local-pipeline-20261011-v2/samples.json)、
[分位汇总](../验收证据/local-pipeline-20261011-v2/summary.json)、
[源码指纹](../验收证据/local-pipeline-20261011-v2/source-digests.json)。
首批保留于 [原始实验](../验收证据/local-pipeline-20261011/manifest.json)：12.58s、510 个阶段无缺失，
但只断言 head 相等和 projection 查询成功，尚未断言正文。复测增加正文/身份精确核验，
并修复 open/setup 失败时提前 panic 丢报告的问题；两批**没有合并**。复测构建期间 mbx
报告另一 CLI bin 源文件被并行修改而未写入缓存，进一步说明本次不是安静、冻结源码窗口。
manifest 含机器/OS/compiler、执行产物 SHA-256 和原始数据 SHA-256。
共享 dirty tree 的源码指纹在运行后采集，不假装它是完整可重建 commit；执行身份以 binary digest 为准。

```sh
source scripts/lib/rust_build_cache.sh
nola_rust_build_cache_init
export NOLA_HOME=$(mktemp -d /tmp/nola-perf-baseline.XXXXXX)
export RUST_MIN_STACK=16777216
export NOLA_BASELINE_OUTPUT="$NOLA_HOME/local-pipeline.json"
nola_cargo test -p nola_cli --test local_pipeline_baseline -- --ignored --nocapture
# 从 Cargo 输出取本次精确测试产物，不假定 hash 永远相同；输出目录必须不存在。
python3 scripts/report_local_pipeline_baseline.py "$NOLA_BASELINE_OUTPUT" \
  target/debug/deps/local_pipeline_baseline-<本次hash> <新的证据目录>
```

此 test 默认 ignored，避免普通单元测试暗中做 60 次性能实验；不能以普通 test 全绿声称它执行过。
生成器拒绝覆盖现有证据目录。原始记录不含用户内容、凭据、机器 hostname 或绝对用户路径。

## 预算决策与仍缺证据

### 会话累计缓存统计的规模基线待补

2026-10-11 只读核对 `committed_run_context_usage::conversation_cache_usage`：当前固定
快照／Run 枚举之外，每个本地 Run 单独查询最新 attempt，每个有缓存快照的远端 Run
还单独验证来源和准入。典型 SQL 数约为 `2 + L + 4R`；这是代码路径分析，不是延迟实测。
Mac `consumeContextUsage` 由事件触发，不能据此宣称固定轮询频率或已证明 UI 卡顿根因。

下次读模型实验应分别控制 Run 数、每 Run 请求数、同请求事件版本数、本地／远端占比，
报告读耗时 median/p95/max、实际 SQL 次数、VM steps、解析字节及数据库行数。
warm read 与连接重开首读分开，失败和未知缓存用量保留；不得把纯 SQL 规模夹具说成
真实 Provider 会话验收。真实远端证明路径可复用 `context_usage_read_e2e` 的准入、
终态接收和后继 Run 流程。当前尚未执行该基线，不设绝对毫秒预算。

潜在优化是一次查询聚合全部本地请求、批量读取远端身份／准入证明，仍保持同一读快照，
本地与远端事实互斥，使用 Rust checked 整数汇总。不能使用 INNER JOIN 将缺失证明
悄悄过滤成漏算，也不能为减少查询去掉接收来源校验。

暂不批准绝对 latency SLO：单机 debug 的单批 30 样本只能提供描述性基线，不足以产生真实 UI、
模型或线上服务预算。可立即建立的规则是保留以上相同口径作为后续变更对照，报告分位变化和失败率，
任何 ACK 丢失/跨会话污染都判失败，不以速度抵销。第二批独立重复、release、长历史、并发，
以及测量开关对照完成后再提交可控阶段预算和统计判定规则。

尚未测：UI submit/IPC；真实 provider 首 reasoning/正文、热缓存；压缩；工具进程/授权；
Mac renderer 首帧及长帧；进程 kill/restart 恢复；长历史 DB；RSS/写放大；断电级持久性。
P01/P03/P07 获得了部分本地阶段依据，P02/P04/P05/P06/P08 不因此达标。

## 回执路径只读分解：下一步测哪里

2026-10-11 在上述实测后核对当前源码；本节**没有再跑 benchmark、修改业务或证明根因**。
已观测的是整个 helper 的耗时，不能把约 50 ms 直接归因于 fsync、恢复或序列化。

实际同步路径：

```text
command JSON 编码
 → execute_received_command: BEGIN IMMEDIATE / Runtime overlay
 → receipt admission / conversation summary / mutation savepoint
 → runtime.execute: hot Core 取出或 cold restore / reducer
 → checkpoint Replay 或 Baseline / transition 与 assistant journal
 → bounded projection signal / receipt result 编码及保存
 → disclosure flush / SQL COMMIT
 → 同步 work archive publication
 → Runtime overlay adoption / ACK return
```

| 核实的代码事实 | 可验证假设，不是结论 | 最小测量点及计数 |
|---|---|---|
| `command_receipt.rs::execute_received_command` 把 Store commit 返回放在 ACK 之前；`connection.rs::begin_write_unit_of_work` 使用 `BEGIN IMMEDIATE` | SQLite writer 竞争可能计入总量；不存在仅从总时间识别锁等待的方法 | outer begin 单独 span；记录成功/失败、busy 分类；不记录 SQL 正文 |
| `connection.rs::open` 设置 WAL、`synchronous=NORMAL`、busy timeout 5s | 5s 是竞争超时上限，不是每次固定睡眠；不能把几十 ms 当作固定 timeout | 将 SQLite COMMIT 本身与其前后工作分开；必要时在隔离库记录 busy handler 等待，不改变超时语义 |
| `runtime.rs` 经 `supervisor.take_current` 验证 Run/session/checkpoint revision；命中时不走 `load_exact_run_checkpoint` / `restore_run_recovery` | 不支持“每次必定从数据库重建 Core”的判断；若 revision 失配则可能冷恢复 | hot hit/miss 及明确 miss 分类；cold restore span + replay record/byte 数；reducer span 另计 |
| `runtime_supervisor.rs::begin_unit_of_work_with_purpose` 建立空 overlay，不复制全部 Core；`take_current` 移动 hot Core | 不支持“事务开始无条件克隆整个线程”的判断 | overlay begin/adopt 单独计时足够，不引入序列化快照来测量 |
| `runtime.rs` 的 ReceiptAck 强制 persist，即使 delta 未达 byte budget；`lib.rs::accept_host_event_in_transaction` 根据 replay readiness 在 Replay 与 Baseline 中二选一 | durable ACK 不等于每 delta 全量 checkpoint JSON。超出 replay 阈值或终态会切回 Baseline，长历史成本可能不同 | checkpoint mode 枚举、encoded bytes、replay count/bytes；编码 span 与 Store transition span 分开 |
| `command_receipt.rs` 对 Host response 使用 bounded invalidation，绕过 before/after 全 conversation projection；仍读取 summary；stream 内容主要用 journal | 已有代码避免旧全历史 projection diff，不能继续把它作为已证实原因；summary/list-change 查询仍有成本 | receipt pre-read、runtime.execute、receipt post-read/encoding 三个不重叠 span，计 query 次数 |
| `connection.rs::commit_write_unit_of_work` 先 `canonical_text_disclosure::flush` 后 SQL COMMIT；flush 每批最多 64 个 pending，但循环至全部完成 | 事务终点含 disclosure 衍生查询/写入；不是只有 SQLite COMMIT | flush span、pending rows evaluated、循环批数；不要关闭 disclosure 校验来比较“优化” |
| **SQL COMMIT 后仍同步调用 `publish_work_archives_after_commit`**；archive dirty 由 checkpoint/body/task/replay 表 update hook 置位 | ACK 可能花大量时间在已提交后的文件归档，checkpoint/replay 热路径也能触发。该假设优先级高，但没有耗时分解支持占比 | SQL COMMIT end→archive begin/end→ACK；archive skipped/attempted/failed 分开，失败仍计耗时 |
| `work_archive.rs::flush_work_archives_impl(false)` 获取阻塞文件锁，再查询全部 checkpoint 身份和全部 durable tasks，逐会话比较 manifest | 归档不仅处理当前 Run，复用库增长或别的 publisher 可能放大 ACK 延迟 | lock wait、checkpoint/task enumeration、manifest checks 三段；计扫描会话数、跳过数、发布数 |
| 对需发布的会话调用 `read_recovery_checkpoint`，`publish` 解析 baseline 并序列化整个 recovery 与六个归档成员；此前 manifest 存在还验证旧 generation 的文件 checksum | 即使 Core 使用 Replay，旁路归档仍可能重读 baseline/replay 并重写整个 checkpoint 归档；不等于重新运行 Core reducer | recovery read bytes/records、encoding bytes、old verification read bytes；分别计时，不能只统计新增 delta 字节 |
| `write_new` 对成员调用 `sync_all`；新 generation 的六个成员、manifest，以及 staging/generations/root 目录均有同步点（目录创建也可能同步） | 多文件同步可能解释非终态响应之间相近耗时，但目前无法确认，也不能把 SQL NORMAL 当成整链路没有 fsync | 合计 file/dir sync 次数及总耗时，区分文件写/哈希/rename；保持同样持久性语义 |

具体入口：
[`command_receipt.rs`](../../../core/nola-application/src/command_receipt.rs)、
[`runtime.rs`](../../../core/nola-application/src/runtime.rs)、
[`runtime_supervisor.rs`](../../../core/nola-application/src/runtime_supervisor.rs)、
[`lib.rs`](../../../core/nola-application/src/lib.rs)、
[`run_effect_port.rs`](../../../core/nola-store/src/run_effect_port.rs)、
[`connection.rs`](../../../core/nola-store/src/connection.rs)、
[`work_archive.rs`](../../../core/nola-store/src/work_archive.rs)、
[`canonical_text_disclosure.rs`](../../../core/nola-store/src/canonical_text_disclosure.rs)。

最小下一步只在相同 60 样本 harness 上增加上述分段的**诊断采集**，优先切开
`runtime.execute / disclosure / SQL COMMIT / post-commit archive`，再按占比下钻；
采样缓存写在整个 ACK 计时之后，保留父子 span 并检查分段之和与总时间残差。
安静窗口、冻结产物后重复，不能把当前并行构建竞争下的总值当作优化前严格对照。
若归档确实占优，也必须先明确 DB authority、派生文件持久化承诺和恢复重建契约，
才能讨论异步/coalescing；本次审计没有授权省略归档、降低同步或更改 ACK 承诺。

### 提交边界问题及异步归档的前置契约（未实现）

事实是 SQL 已 COMMIT 后再同步尝试归档；失败只记录“database commit is safe”并重新置内存
dirty，成功和失败都在 ACK 返回前等待。这意味着目前 ACK 延迟包含一个不决定 DB 提交成败的
派生工作，却也**不保证归档成功**。不能把现有 ACK 向用户解释成“数据库与文件原子提交”。
dirty 目前是进程内 AtomicBool；open 会初始化为 true，可触发后续补扫，但不是按会话持久化
的交付记录，也不能单独证明无新命令时归档能在有界时间内赶上。

若后续证据支持异步化，至少先落实这些前置条件，不是简单删 `sync_all` 或 `spawn`：

1. 数据库仍是业务状态 authority，归档是带版本的可重建派生视图。明确 ACK 承诺数据库还是
   同时承诺文件；若用户显式要求“导出完成”，应单独等待具体归档版本，不复用一般命令 ACK。
2. 在业务事务内持久登记每会话目标 frontier/revision（或以可证明完整的 canonical cursor
   派生 backlog）；持久记录 published frontier、失败/重试状态，不能只用内存 dirty。
3. 单一归档执行所有者，从已提交一致性快照生成；coalescing 只可合并同会话较旧目标，不能
   丢最新目标、跨会话覆盖或把已经删除的要求重新发布。归档中的 Goal/Task/压缩数据同属该快照。
4. 发布仍需 generation/checksum/原子 manifest 及版本防倒退。慢旧 worker 不得覆盖新版本；
   file rename 完成但 published ACK 未落库时，恢复可核验并幂等确认，不能误删新 generation。
5. 覆盖崩溃窗口：DB commit 后未执行、写 staging 中断、manifest 切换前后、状态登记前后、
   disk-full/锁超时/权限失败；重启扫描持久 backlog，不能依赖再次聊天才修复。
6. API/UI 显示 archive target/published revision 与 pending/failed，错误不改写原命令的
   已提交结果；给归档滞后设**先测再定**的预算、容量和重试/退出策略，停机不能无限静默丢任务。
7. 用故障注入证明 DB ACK 不丢、文件不发布未来/旧版本、重启最终能重建，以及文件消费者
   不把旧 snapshot 当最新 Goal。完成这些条件前，不能以降低延迟为由改变现有行为。

## v3：最小提交分段已执行（无性能优化）

本轮实现 [commit_diagnostics](../../../core/nola-store/src/commit_diagnostics.rs)：默认关闭、同线程
显式 capture、最多 128 条记录，溢出计 dropped；关闭时不分配观测记录、不读取时钟。
只记录枚举阶段/结果与单调耗时，无正文、路径或同步诊断日志。capture Drop 在 unwind 时禁用，
后续追加 generation 绑定防止旧 Timer 落入新 capture。未修改事务、归档、错误处理、同步或 ACK 语义。
归档调用内部记录 Failed/Skipped，不能因为外层 DB commit 返回 Ok 就误记归档成功。

新证据：[manifest](../验收证据/local-pipeline-20261011-v3/manifest.json)、
[raw](../验收证据/local-pipeline-20261011-v3/samples.json)、
[总耗时](../验收证据/local-pipeline-20261011-v3/summary.json)、
[分段](../验收证据/local-pipeline-20261011-v3/segments.json)。
60/60 会话正文和身份核验通过；510/510 父阶段；执行 13.75s，命令 exit 0。
300 个 receipt 各有 3 个预期分段，即 **900/900 receipt 子阶段齐全**；
另外 open/reopen 内观察 120 个 archive 调用，全体原始观测 1020 条，dropped=0。
总体结果：disclosure 成功 300，SQL COMMIT 成功 300，archive 成功 330/skipped 90/失败 0；
没有把 skipped 填成成功。诊断 on/off 的额外耗时尚未通过交替对照量化。

只展示首次 1 KiB 正文 receipt，单位 ms，median / p95：

| 部分 | 新库 | 复用库 |
|---|---|---|
| 总 receipt | 51.066 / 54.083 | 53.169 / 55.847 |
| disclosure flush | 0.007 / 0.008 | 0.007 / 0.008 |
| SQL COMMIT | 0.972 / 1.196 | 0.834 / 2.464 |
| post-commit archive | 41.925 / 44.386 | 44.191 / 46.486 |
| 未分解余量 | 8.350 / 9.887 | 8.579 / 9.022 |

余量是**逐样本**从父耗时减去三个不重叠子阶段，脚本拒绝负残差，不是各项 median 相减。
余量包括业务变更、receipt/JSON、运行态 adoption 和诊断观测成本，不能命名为 Core reducer 时间。
逐样本 archive/total 比率 median 为 81.82% / 82.46%。本 fixture 现在有直接分段证据支持：
同步归档占 receipt 大部分时间；仍不知道其中文件 sync、扫描、编码各自占比，不外推真实长会话。
**v2 与 v3 不能用作优化前后对照：本轮只增加观测，没有实施优化**；构建身份、竞争和测量开销
均可能变化。下一步优先验证派生归档的提交边界契约，不以移除同步或丢数据实现“提速”。

诊断契约测试：`nola_cargo test -p nola_store --lib commit_diagnostics:: --quiet`
最终 6 passed、367 filtered、exit 0，0.05s。覆盖关闭无记录/时钟、容量上限与 dropped、
Failed/Incomplete 区分、线程隔离、unwind 清理、capture generation 隔离、真实 deferred-FK
SQL COMMIT 失败不进入 archive，以及真实归档路径为文件导致归档 Failed 但 DB commit 仍成功、
重开 checkpoint revision 保留。追加 generation 时第一次编译缺 Result 错误类型推断，修正后通过。
v3 采样运行于 generation 加固之前的产物，保留该产物独立 digest；加固后的 6 项契约测试
不是重测 v3 性能，也不伪称二者同一二进制。

## v4：数据库 ACK 与派生归档分离

实现范围：Store schema 62（epoch 4 不变）。新增单行 `work_archive_publication`，通过
checkpoint/body、durable task、Host replay 源表的 INSERT/UPDATE/DELETE 触发器在**原事务内**
增加 target generation。 published generation 只在归档一致性快照发布成功后确认。
61→62 是非破坏升级；迁移保留 checkpoint，published 从 0 开始，不能假定旧文件已校验。
正常创建/升级在 `BEGIN IMMEDIATE` 内执行，锁后重读版本，防止并发 open 的过期 schema 判定。

普通 Store commit 在 DB 提交后只向有界通道发非阻塞 wake hint，不再发布文件。进程内按
规范化 DB 路径共享一个 worker；worker 不持有自己的 sender，最后一个拥有该 Store 的连接关闭
后通道断开，工作线程结束（正在进行的文件 I/O 仍需返回）。不为每条消息 spawn 新线程。
后台连接不做 prune、disclosure 或其他业务修补，仅读取已提交业务快照和更新派生交付状态。
跨进程/显式导出通过同一文件锁串行发布：**后台 try_lock**，不等待其他 publisher；显式 flush
可等待最多 5 秒取得锁，失败返回 busy，不无限阻塞。文件锁获取后才 pin SQLite read snapshot。

后台每秒检查持久 backlog；失败持久记录固定类别（不含路径/正文）、次数、下次重试时间，
退避 1/2/4/8/16/30 秒，最大 30 秒。没有新聊天也会重试。临时 wake 丢失或进程退出不丢目标；
下次 open 自动重新注册 worker。不能保证进程已经退出时继续导出；需要文件完成的调用方应显式
`flush_work_archives`，其返回与普通聊天 ACK 是不同承诺。

`work_archive_status` 现在返回 `publication` 全库状态（target/published/last error/retry），
原会话 pending 仍按该会话 checkpoint/task 与文件内容核对，不能把全库失败标签当作当前会话失败。
缺少 owner checkpoint 时拒绝宣称旧文件 available；扫描发现已删除 owner 后写持久 retired tombstone，
`read_verified` 拒绝恢复该旧 Goal，但保留旧 manifest 和 generation 字节，不删除恢复资料。
重新出现合法 owner 时，只在新 manifest 安全发布后移除 tombstone。内存库没有文件成功承诺。

此实现采用**全库 generation/扫描**，不是最优成本的每会话 outbox：每次实际导出需扫描所有
checkpoint/task 并比对各 manifest，只重写变化的会话。coalescing 的输入是持久目标而非内存 dirty；
DB 在文件发布期间继续增长时，只确认当次快照 generation，新的目标留待下次，不能提前宣称完成。
多次目标可能合成一次发布，但重试、缺失和错误不能合成成功。

并发暴露了一个既有写入边界缺陷：`create_runtime_task_with_event` 曾用裸 SAVEPOINT 在无外层
事务时先读后写，后台短状态 UPDATE 可触发 SQLITE_BUSY_SNAPSHOT(517)。现复用 WriteTransaction
RAII（外层 BEGIN IMMEDIATE、嵌套 savepoint），拒绝/异常由统一 guard 清理，未增加 busy timeout。
新增重复任务拒绝后连接及调用方外层事务仍可正确使用的测试。

### v4 实测与验收边界

独立证据目录：`evidence/local-pipeline-20261011-v4/`。仍是同一两条件各 30 次的
真实 Application→SQLite fixture；不调用 Provider，不访问生产配置。单测运行 4.36 秒，
60/60 样本、510/510 父阶段成功，失败 0、missing 0；300 个 receipt 各有 disclosure/SQL
两条诊断，共 600/600，dropped 0。普通 ACK 线程 `post_commit_archive` 观测数和预期均为 0：
后台线程没有开启 TLS capture，不能把 0 当作后台归档耗时为零，也不能把后台耗时混入 ACK。
report 脚本按原始 `commit_path` 判别 2/3 段，旧 v3 数据仍可独立复算。

| 首次 1024B 正文阶段（ms） | fresh median / p95 | reused median / p95 |
| --- | ---: | ---: |
| receipt 总计 | 6.929 / 24.315 | 5.325 / 36.477 |
| disclosure flush | 0.005 / 0.006 | 0.005 / 0.009 |
| SQL COMMIT | 0.766 / 1.141 | 0.530 / 11.149 |
| 未分解余量 | 6.109 / 21.022 | 4.740 / 27.748 |

这是首次正文提交，不是持续流式吞吐指标。v3 的同步 archive 在 ACK 内，v4 由代码边界和
分段证据共同证明不再位于 ACK 内；**不能把两个运行的数字比值当稳定提速倍数**。
不同产物、共享工作区并行构建、后台归档竞争及观测成本均可能改变结果，v4 长尾仍明显。
源文件 manifest 在运行后采集，二进制 SHA 才是已执行产物身份；此后新增的 cfg(test) worker
生命周期测试不属于该性能产物。没有测真实 Mac 首帧、真实模型首 token、断电耐久性，
也没有据此提高任何生产预算。

专项测试：runtime_task_e2e 32/32（并行），work_archive_e2e 5/5，
work_archive_async_e2e 3/3（另有 1 个仅供父测试执行的 ignored 子进程 fixture）。覆盖持锁时
DB ACK 成功、归档持久失败与无新写入重试、dirty 回滚、删除 owner 拒绝旧档恢复，以及真实
子进程写入后直接退出、重开自动发布。Store lib 新增生命周期测试验证外部 publish 文件锁仍
占有时，最后 Store 关闭使 worker 正常退出；61→62 fixture 验证持久目标从 pending 重建。
完整 Store 最终复跑：374 passed / 0 failed / 1 ignored（23.19 秒，4 个测试线程）。此前
三项失败已处理：新建/迁移使用一致的模型选择索引 DDL；迁移 fixture 明确覆盖新增归档对象；
BM25 同分在 limit 前补齐 part ordinal，展示按消息与 part 排序，保留原结果断言。
该轮使用隔离 NOLA_HOME、RUST_MIN_STACK=16777216；不据此声称默认线程栈问题已解决。

后续统一验收（不是重跑 v4 性能）：schema 63 输入来源索引/后继 Run 集成后，完整 Store lib
再次通过 **383 passed / 0 failed / 1 ignored，34.36 秒**；Application 输入来源专项 **3/3**。
v4 仍对应原 schema 62 性能产物，不能把最新验收集合追溯当成该旧二进制的测试集合。
没有新增性能数字或 UI/Provider 提速声明。

## Schema 66 Store 长正文阶段样本（2026-10-11）

本轮直接运行上一轮已通过 405 项 Store 回归的测试产物
`target/debug/deps/nola_store-5f10ca9ed17e578b` 的显式 ignored 用例
`canonical_stream_performance_tests::production_stream_2_8_32_mib_disk_acceptance`。
执行 exit 0，1/1 通过，15.25 秒；每种尺寸只有一个样本，不计算 p50/p95。
机器实查仍为 Mac17,3、16 GiB、10 logical CPUs、macOS 27.0.1 (26A434)、Rust 1.99.0。
debug/unoptimized 产物，16 MiB 测试线程栈，隔离 NOLA_HOME/临时库；无真实 Provider、Core、App
或 UI。运行时没有本轮 Cargo 构建，但未控制其他系统负载。此产物早于后续 Domain commitment
修改，不应当作新 Domain 协议的性能数据。

每次追加 32 KiB，真实 `commit_transition`，checkpoint 固定为 `{}`；包含应用校验、SQL 和
事务，但不包含真实 Core checkpoint 序列化。关闭测量同时包含 Stream Close 与 Part Close。
尾部测量为 100 次 tail + range 组合读取。重开后逐段重新计算全文摘要并核对原字节。

| 正文字节量 | 追加次数 / 总 ms | 关闭 ms | 100 次 tail+range ms | 重开后全文摘要 ms |
|---|---|---|---|---|
| 2 MiB | 64 / 410.315 | 202.215 | 228.217 | 104.816 |
| 8 MiB | 256 / 1548.813 | 805.414 | 229.314 | 415.886 |
| 32 MiB | 1024 / 6115.513 | 3196.405 | 224.841 | 1659.368 |

最终 DB/WAL 字节分别为 5,124,096/4,239,512、19,075,072/5,805,112、
70,074,368/10,880,952；page size 4096，journal_mode WAL。SQLite cache page writes
追加分别 2530/10241/41513，关闭 20/23/23；这些不是 OS 写字节或 fsync 次数。
测试输出字段 `cold_complete_digest_ms` 只表示连接重开后的摘要读取，没有清 OS page cache，
这里不称真正冷盘读取。临时目录按每种尺寸生命周期清理，未修改用户库。

观察：尾部限量读取在本组样本中接近固定成本；关闭耗时随总正文大小增长，32 MiB 已达约 3.20s。
源码 `canonical_stream_store::apply` 的 Close 分支执行 `require_complete`，存在全文完整性检查；
不能据此把整个关闭耗时归因于这一处，更不能删除校验换速度。下一步应分别 profile 完整性检查、
Part Close 派生工作与事务提交，并补 release/重复样本后确定预算。当前证据不能证明 UI 卡顿原因、
发布版吞吐或最终 seal 延迟，也不证明整个架构性能达标。

### 关闭路径去重后的对照

沿用以上机器、fixture、debug 构建与 32 KiB 块。优化前再跑两次，加上原始一次共三次；
优化后冻结的 Store 产物跑三次，均完整校验并通过。未交错随机化顺序，不作因果统计或 p95 承诺。
测量时无本轮编译同时运行；正常后台负载未隔离。产物包含关闭优化，但早于其后的单版本 schema
清理；不能把这些性能数据追溯给尚未测试的新初始化流程。

| 尺寸 | 优化前关闭 ms（三次） | 优化后关闭 ms（三次） | 中位数变化 |
|---|---|---|---|
| 2 MiB | 202.215 / 200.050 / 202.288 | 144.658 / 143.391 / 143.494 | 202.215 → 143.494 |
| 8 MiB | 805.414 / 796.353 / 802.164 | 569.796 / 565.784 / 565.203 | 802.164 → 565.784 |
| 32 MiB | 3196.405 / 3201.752 / 3203.432 | 2287.955 / 2265.912 / 2280.675 | 3201.752 → 2280.675 |

本组关闭时间中位数下降约 29%，不是整个请求延迟下降 29%。追加与限量尾读路径未改。
32 MiB 关闭的 SQLite cache misses 反而由约 10.8k 升到 18.4k；不能宣称 I/O 改善。
cache writes 仍为 23，未删完整性检查或降低持久性。重复 run 的总用例时间分别为优化前
15.25/15.22/15.15s、优化后 14.16/14.06/14.17s。

实现边界：`canonical_stream_read::audit_complete` 在同一函数固定快照中完整校验所有代次
manifest，只缓存当前代次已核头部/前驱元数据，再校验当前正文块归属、连续偏移、长度及前驱；
`complete_digest` 同遍追加 SHA。Part Close 不再逐个 range 重做 manifest 验证。Stream Close
与 Part Close 仍各自核验，不跨 mutation 或事务保存“已验证”标志；后续事务篡改必须重新拒绝。
空间成本是 O(当前代次 manifest 数)，不是常数；不缓存正文或旧代次元数据。极多小块下的
内存峰值、发布构建和真实 UI 仍需测量，不将此取舍隐藏为无成本缓存。
