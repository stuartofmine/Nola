# Terminal-Bench 4.0 五题开发预算实测（2026-11-09）

五题均完成真实模型执行和官方评分，**0/5 通过**。五题没有 Harbor trial exception，均在每题 1800 秒的外层上限前结束。HTTP 200、内部 answered 终态以及评分包装测试通过均不代表任务通过；本报告只用官方 reward 判断通过。

| 用例 | 官方 reward | Agent 执行时间 | 模型尝试数（含子任务） | 压缩完成次数 | 官方验证表现 |
| --- | ---: | ---: | ---: | ---: | --- |
| embedding-drift-monitor | 0 | 112.52 秒 | 15 | 0 | 3 项验证通过、8 项失败；失败项报告降权 worker 未返回成功。 |
| foodstuff-beta-activity | 0 | 133.19 秒 | 19 | 0 | 未生成要求的 results.txt；13 项验证失败。 |
| freight-dispatch-shift | 0 | 1683.94 秒 | 219 | 3 | 生成 dispatch；诊断 151/232（0.6509），官方 reward 仍为 0。 |
| html-js-filter | 0 | 20.95 秒 | 3 | 0 | 未生成要求的 filter.py；2 项验证失败。 |
| vf2-speedup-networkx | 0 | 512.38 秒 | 69 | 0 | 60 项验证失败，输出多处为降权 worker 未返回成功。 |

这是一轮预先固定五题的开发样本，不是完整榜单成绩。[官方 4.0 协议](https://www.tbench.ai/news/terminal-bench-4-0)的每题预算为 8 小时；本轮经用户选择使用每题 30 分钟开发预算、串行执行、max_iterations=0（不设模型轮次上限），不能直接与官方榜单比较。

## 冻结条件

- 上游：harbor-framework/terminal-bench，4.0.0，固定 revision `452bf305c6daa62fc59061d22133a7cbc7c1572e`。五题取自仓库现有 `v4.0.0-nola-20` 固定集合，原始任务、Dockerfile、评分器未改动；没有向模型提供评分诊断或参考答案。
- 模型：当前选中的 `gpt-6.1-terra`；声明 context_window=1,050,000、output_limit=128,000；没有另设 reasoning effort，没有人工缩小窗口。真实凭据只存在宿主机转发器内存中，容器仅持有无实际权限的 relay token。
- Linux Rust 1.99.0 构建。nola SHA256：`7bd4a3db6fcd7879c1c1c1f2ad8dd1291f070aecdba590cce8222545260d93d7`。运行前后均核对二进制和五个执行/证据脚本哈希一致。源码文件哈希清单、构建日志及二进制均在本地证据归档。
- Harbor 0.22.0，Docker Desktop Linux/aarch64；串行，每题 2 CPU、4 或 8 GiB 内存，0 GPU、10 GiB Agent 存储。
- 隔离任务容器显式设置 `NOLA_PERMISSION_PROFILE=allow_all`；不会授权宿主机工具或挂载真实用户数据。适配器原先仅有 `--yes`，该参数不授予工具权限；本轮修正了适配器及对应测试。真实执行期间没有修改产品代码或执行脚本。

## 请求与压缩

转发日志记录 **325 次生成请求、325 次 HTTP 200、0 次传输错误**。逐条核对模型名一致、上下游请求 body SHA256 一致，未注入回复、错误或压缩中断。

提供商报告的累计输入 15,048,260 tokens、输出 124,284 tokens，其中缓存输入 4,791,595 tokens（累计输入的 31.84%，不是最后一次请求的缓存命中率）。累计输入包含各次重复上下文，不能当作单个上下文大小。

调度题记录 3 组 compaction_requested / compaction_completed，随后仍有模型调用；其他四题没有压缩完成事件。这里的“3 次”来自完成事件计数，不是把请求和完成两类事件相加为 6 次。本轮证明压缩路径实际执行过，尚未计算每次压缩前后完整请求的占用比例，不能据此声称达到 50% 目标。多层任务可能采用不同上下文预算，不能用根会话声明窗口直接解释每个压缩触发点。

## 暴露的问题与证据边界

五题都出现 `task_workspace_preparation_blocked`，并出现 `dedicated_tool_required`。即使任务容器采用 allow_all，托管工作区准备仍被阻止，说明权限策略与工作区准入并非同一条执行链。需要独立复现、修复通用工作区契约后重测；不能单凭这些错误认定五题的全部失败都源于权限。

两题很快结束且缺少交付文件；食物活度题内部为 skill.acceptance_pending。调度题运行约 28 分钟，包含大量子任务及压缩，但仍有 verification.pending 终态和实际验证缺口。监控题和 VF2 题的降权 worker 失败尚未确定是产物权限、可导入性还是算法实现问题；本轮不修改官方评分器，也不把未确认原因归咎于环境。

数据库导出中的 `seal_digests_verified=false` 是当前导出器明确标注“未执行 seal 摘要校验”，不是发现了摘要不匹配。因此内部终态只作辅助诊断，不作为经过完整性校验的成功证明。官方 reward 来自每题 verifier 输出。

## 初始失败、验证与归档

最初一轮五题全部在模型启动前失败：临时配置错误使用了不受支持的 inline api_key。随后一次配置预检因部分 capabilities 对象缺字段失败。两次均无生成请求，保留原始记录，**不计入本轮五题模型失败率**。修正临时配置为 CLI 支持的 api_key_env 并保持用户模型声明后，Docker 配置及 relay 可达性预检通过，执行本轮五题。

Linux 首次使用 Rust 1.91 构建因源码所需特性失败；安装 Rust 1.99 后重新构建成功，并完成容器 CLI smoke。适配器、运行器、聚合器共 37 项单测通过；benchmark 清单校验通过 2 个集合 / 40 个用例，反特化检查通过 158 个字面量。本轮控制器返回 0 表示执行完成，不能代替题目通过率。结束后已移除临时模型配置并停止 relay。

本地完整归档：`.artifacts/evals/terminal-bench-runs/dev5-20261109-5g0ns95g/evidence.tar.gz`（已被 gitignore 忽略；包含初始失败、构建及当前真实执行证据，不提交）。同目录保留 selected-five-report.json、campaign.json 与 aggregate.json/md。通用聚合器按既有 20 题清单统计，只有这 5 题实际执行，另外 15 题未执行；不能把聚合器补零结果当作完整 20 题实测。

原始临时目录：`/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-tb4-five-20261109-5g0ns95g`。本报告不包含原始任务文本、参考答案、凭据或本机业务数据；本轮未提交或推送。

后续逐题因果、Git 最小复现与根会话压缩前后占用分析见 [轨迹分析](TERMINAL_BENCH_4_TRACE_ANALYSIS_20261109.md)。该分析进一步确认监控题未修改生产模块、VF2 未集成交付，并补齐一次压缩前后完整请求的实际 token 数据。
