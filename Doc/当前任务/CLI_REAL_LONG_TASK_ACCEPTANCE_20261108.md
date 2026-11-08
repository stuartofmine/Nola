# CLI 真实模型长任务验收

日期：2026-11-08。模型：用户当前默认连接 `gpt-6.1-terra`，OpenAI Responses。产品可执行文件 SHA-256：`f33e502c154c22635fa8731fb9fe6997bf4f4c1fb4588394d0ef2e8b28d56a54`。本文只覆盖所列 CLI 工作流，不关闭全平台 UI、其他 provider、移动端设备或全部长任务验收。

## 结果

共发出 **117 次真实模型请求，全部 HTTP 200**。其中最终通过的六类场景使用 88 次请求，其余 29 次属于初次覆盖尝试和测试断言修正前的运行，未冒充通过。除 HTTP 状态外，还核对 canonical Run 终态、实际工具调用、数据库、生成文件以及独立预期结果。非权限场景均观察到真实文本 delta 和正常流结束；权限用例在工具授权等待处停下，不要求生成最终文本。

| 最终场景 | 真实请求 | 验证结果 |
| --- | ---: | --- |
| 多文件数据处理与错误恢复 | 11 | 缺失文件读取失败后继续查明输入；生成程序/测试/说明并执行；独立扩展的 12 组检查通过，覆盖大数金额、跨文件去重、重复执行与错误输入不覆盖已有输出 |
| 连续依赖工具链与冷启动续跑 | 23 | 16 个随机文件节点逐个解引用，汇总与校验值准确；新 CLI 进程用英文追问，结果准确，输入文件哈希未变 |
| 私有记忆与冷启动 | 11 | 两个会话交替保存/搜索随机私有标记；持久记录 owner 正确，续跑各自只返回本会话标记 |
| 覆盖权限等待 | 2 | 原文件不变；Run 保持未封口的持久等待；新进程查询仍有待批准请求，没有伪报完成 |
| 实际上下文压缩与续跑 | 28 | 注入两轮合成历史后执行长链；记录到 **2 次非空真实压缩**；冷启动续跑计算正确，随后准确返回仅出现在压缩前历史中的随机事实 |
| 后台任务与归属拒绝 | 13 | 60 秒任务只启动一次；其他会话 read_task/stop_task 均失败且未泄露输出；原会话新进程等待同一任务完成，六步进度及最终结果正确 |

后台任务三轮耗时约 15.90、15.38、42.51 秒；多文件开发轮约 96.16 秒。依赖链是受控的多轮工具压力测试，不能等同于任意复杂开发项目。记忆用例采用交替会话调用，并未宣称两个模型请求并发执行。

本轮没有确认新的产品缺陷；发现并修复了验收脚本的当前合同假设：缓存诊断应来自冷读 projection，ModelAttemptUsage 只对照原始 cache_usage/cached_input_tokens 等字段；任务类型应为 shell_process；权限等待应检查 Application projection，不能要求旧物理 permission_requests 表出现记录。新增对应缓存反例和 fixture 隔离检查，18 项通过。

首次压缩 fixture 没有达到逻辑阈值，明确保留失败 receipt，随后改用合成历史触发真实压缩。多文件独立检查曾把未定义的 pending 状态当作非法输入；已改用明确的未知状态，不将该测试歧义记成模型错误。

## 缓存与安全隔离

全部运行累计 provider usage：input 2,908,974 tokens，output 11,085 tokens，cached input 1,363,473 tokens。计数包含上述初次尝试，不是价格估算。已对四份真实冷读 projection 重审缓存累计值、attempt 归属与原始 usage，一致；数据处理轮和依赖链累计命中率分别约 55.20% 和 66.53%。单次请求仍可能真实未命中，不能仅凭最后一次为 0% 判定错误。本文没有验证终端缓存组件的实际视觉显示。

测试通过透明本地 relay 原样转发选定模型的 Responses 生成请求，认证只在 relay 内存中加入；CLI/模型子进程不接收 provider key。relay 拒绝并不转发目录探测，报告不把这些本地 403 算成 provider 失败。没有伪造模型输出、注入生成错误或模型 fallback。fixture 显式声明所需能力，不用模型名推断能力。

每类测试使用独立项目、NOLA_HOME 与数据库；不修改用户配置、用户库或凭据。允许的文件和命令仅用于拥有的测试目录；权限测试另用 cautious profile。关闭自动记忆维护/驻留回忆，以核对显式 owner 合同；这不覆盖全局自动记忆提取。所有保留文件（包含 SQLite）已按现有 key 值扫描，无泄露。

## 复现与证据

入口：[test_real_cli_long_tasks.py](../../scripts/test_real_cli_long_tasks.py)。默认执行全部六类，也可用 `--case pipeline|chain|memory|cautious|compaction|background` 单独运行。需要当前格式配置中的默认 HTTPS Responses 连接和既有环境变量凭据；每次执行会产生真实模型费用。

```sh
python3 scripts/test_real_cli_long_tasks.py --case all
PYTHONPATH=scripts:scripts/e2e python3 -m pytest -q \
  scripts/e2e/test_long_task_fixture_safety.py \
  scripts/e2e/test_real_file_permission_audit.py
```

本机隔离证据位于 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/` 下，临时目录可能被系统清理；不将数据库、凭据或原始日志提交仓库。汇总文件为 `nola-real-long-summary-20261108.json`。最终六类目录分别为：

- pipeline：`nola-real-long-1tod1c3y`（扩展独立检查见该目录中的 oracle receipt）。
- chain：`nola-real-long-c5j5l3an`。
- memory：`nola-real-long-4yt6bavs`。
- cautious：`nola-real-long-3fkxh_aq`。
- compaction：`nola-real-long-yiy77vg5`。
- background：`nola-real-long-y0uop173`。

仍未覆盖：PTY 内实际点击/键盘批准及权限引导可用性、图片粘贴、终端视觉、断网与 provider 限流、Android/iOS/macOS UI 和真机执行。不能由本轮权限等待用例推定这些问题已经解决。
