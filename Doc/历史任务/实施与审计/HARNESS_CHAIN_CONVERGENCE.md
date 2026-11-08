> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Harness 全链路契约收敛

日期：2026-10-13。状态：本轮契约与投影整改、静态审计和代码回归完成；任务质量未达标。最终完整 LAN 三题（v9）0/3，验证诊断补丁后的定向单题（v10）仍为 0。真实压缩性能和全客户端人工验收不包含在通过声明内。旧测试结果不作为本轮完成证据。

## 三题轨迹复盘后的控制面减负（2026-10-13）

### 当前实现：Chain-v8 / v9 / v10

模型侧 `requirements_update` 现在只负责最终结果：`outcome`、`answer`，
完成实际动作时附上 `assessments[{requirement, verdict, rationale, evidence}]`。
模型使用当前请求的 `rN/eN/fN` 短引用；Core 把引用解析为已提交的要求、观察和阻塞事实，
再生成内部原子 assessment/result 事务。模型不能传 operation、revision、scope 或伪造证据。
普通回答不要求调用该工具；不完整结果无需制造工具调用来收集证据。
这是替换接口，不保留前一版模型参数。内部审计、来源验证、权限和结果仲裁没有删除。

请求绑定中持久化引用映射与输入 frontier，Core checkpoint 为 24。
任务观察以显式非用户请求的数据消息投影；不再把每条 observation 的 Host 来源元组写进 System。
引用仅对当前请求有效；过期响应仍被原有 scope/frontier 校验拒绝。

策略反馈记录最近八次实际活动，以及连续写入和相似 payload 计数。
相似度只用于建议：排除 schema 声明的资源字段后比较内容摘要/有界行样本，
没有语言关键词、任务名称或注释解析规则。连续六次写入或三次相似写入会建议
检查真实诊断、明确未解决条件；正常批量/模板写入仍可继续。
该信号不等于“没有语义进展”，不拒绝工具、不扩大恢复预算、不授予权限、不清除验证义务。

评测器设置 `NOLA_EVAL_REQUEST_AUDIT_DIR`；正常运行不设置，因此不会额外落盘会话正文。
公共发送边界记录每次准备发送的实际 JSON body（包括随后失败/取消的尝试），
保存原始摘要和脱敏后摘要，去掉 headers、URL userinfo/query/fragment，
并替换配置凭据、实际认证头（含 Basic）、URL 认证值、敏感字段及其正文回显；文件权限为 0600。
配置凭据必须独立传入，不能只从构建后的认证头反推：URL Basic 认证可能覆盖显式 Bearer 头。
记录声明 `provider_receipt_proven=false`，它证明客户端准备发送的内容，不证明服务端收到。
脱敏会改变正文，不能用脱敏后 body 逐字节验证原始缓存命中；原始摘要保留关联。
记录失败只输出不含请求数据的诊断，不改变主请求执行结果。

本轮统一回归：Core 单元 517、Host 契约 147、最终 CLI 单元 1801 通过 / 4 忽略，
CLI 117.79 秒；日志 `/tmp/nola-harness-v8-core.log`、`/tmp/nola-harness-v10-cli-final.log`。
Application / Store 另补持久化回归：315 / 409 通过，各忽略 1；
日志 `/tmp/nola-harness-v9-persistence.log`。本次 Rust 合计 3189 通过 / 6 忽略。
评测导出 / Harbor / supervisor 的 Python 回归分别 9 / 6 / 3 通过。
首次归档回归发现 URL Basic 覆盖 Bearer 后凭据回显未脱敏，已修复，未降低断言。
CLI 最终回归包含该回归以及 Host 首次诊断投影的两项新测试。大 unwind section 链接警告仍存在。
真实三题结果单独记录在 LAN 验收文档，不能把上述协议测试当作任务得分。

Chain-v8 的真实请求归档进一步证明：Host 已经把大型 shell JSON 按开头裁剪时，
Core 无法恢复被命令源代码挤掉的 stdout/stderr。Chain-v9 将该投影移到 Host 首次裁剪处，
本地及外部工具入口共用：保留退出事实、stdout/stderr 的首尾，完整 JSON 仍单独保存。
选择字段后的预览不是原文件的连续字节，不提供虚假的 `preview_end_byte_offset`；
使用明确的 selected-shell-fields 标记，按完整引用搜索或从零读取。原文本预览仍保留准确字节续读。
即使保存失败，也保留诊断并明确结果可能已经产生副作用，不以缺少完整输出诱导重做。

Chain-v9 复跑还暴露了验证后的诊断分离：目标在验证过程中变化时，Host 只给 presentation
追加失败原因，canonical data 仍只显示 exit=0。Chain-v10 将验证证据不可用的状态、错误码、
原因写入结构化 data，并在 Host 裁剪后保留；真实进程退出码不改写，实际副作用不抹掉，
无效 checks 不保留。目标变化和快照失败共用这个出口，不按诊断关键词决定结果。
新增跨层回归覆盖两种原因及大型输出裁剪。首次测试编译的 Option 类型错误已修正；
最终 CLI 全量通过。三个 v9 成绩及额外 v10 单题验收分开记录，不能混算。

v10 单题在 600 秒后正常取消并封存；该随机运行没有再次触发验证目标变化分支，
不能宣称新的失败诊断已被真实模型触发验收。确定性跨层回归已验证该分支；
真实请求则验证了诊断预览、正常执行和取消封存。全部成绩和限制见
[LAN 验收记录](../../../evals/benchmarks/terminal-bench/LAN-20261013.md)。

### 历史批次：Chain-v7（已由上面的模型协议替换）

前述机制验收不代表任务质量验收。Chain-v6 三题均未交付要求的产物；
不能以执行封存或防重复测试通过宣称 harness 已解决空转。

本次落地的边界调整：

- `requirements_update` 模型参数只保留 `operations`、`answer`。
  scope/revision/input frontier 由 Core 在模型请求发出时冻结并随 checkpoint 持久化，
  接收控制结果时使用该绑定执行原有并发检查，不使用响应到达时的新状态补签。
  模型传入身份字段明确拒绝；不保留旧模型接口。Checkpoint schema 为 23。
- 收尾阶段禁止的是新增外部动作；已提供的控制工具仍可单独提交最终结果与正文。
  提示和工具可用性不再互相矛盾。
- `write_file` 统一为 literal `content`，删除 `json_content` schema 和执行分支。
  保留 dry-run、摘要校验、权限、容量和原子写入保护；局部修改使用 edit。
- Core 在结构化命令输出中优先投影 stdout/stderr，而非显示标签。
  截断保留首尾，JSON 编码后总预算继续受限；退出码和完整输出引用独立保留。
  不根据诊断文字关键词推断成功或失败。
- 隔离评测导出 schema 3 增加 assistant journal 的 heads、records、parts、
  manifests、chunks 和关联索引；二进制用 hex 无损保存，不把不同 generation 拼接。

该批次当时未完成、由 Chain-v8/v9 继续实现的主线：模型侧 assessment/证据引用进一步简化；跨步骤策略恢复；
完整脱敏物理请求归档（journal 不等于 provider 请求）；真实三题重新运行与逐轮分析。
本次没有增加语义进展分类器，也没有声称改文件名或改版本注释的循环已被解决。

本批集中回归：Core 单元 515 通过，Host 契约 146 通过，CLI 单元
1797 通过 / 4 忽略，评测导出 9 通过。CLI 全量使用
`RUST_MIN_STACK=16777216`，最终运行 116.64 秒。首次默认栈运行溢出；
扩大栈后的首次全量发现 Host 批准标记误拒绝和旧测试参数，修正后全量通过。
期间一轮并发测试出现 attempt 时间倒退等失败，后续完整复跑未复现，
不能将其宣称为已修复的生产时钟问题。链接器仍报告大 unwind section 警告。
未重跑真实模型 benchmark，不能据此宣称通过率或缓存命中率改善。

## 所有权与流程

模型调用 → Core 规范调用 → 权限/重复/预算准入 → Host 执行 → 结构化执行结果 → Core 状态转换 → Application 原子提交 → 客户端投影。

模型请求由 Host 统一组装稳定政策、历史和动态事实，再由 provider 适配器生成实际请求。压缩以实际请求快照为依据；路由、容量、媒体身份必须贯穿持久化与恢复。

| 边界 | 输入 | 输出及不变量 | 失败处理 |
| --- | --- | --- | --- |
| 调用准备 | 模型参数、工具 schema | 唯一规范参数；去重、依赖登记与执行使用相同身份 | 不完整参数不得执行；错误不改写原操作语义 |
| 执行结果 | 已验证工具协议和执行回执 | 主链与 delegated 共用 NotStarted / Completed 等解释 | 业务字段不能覆盖结构化执行状态 |
| 请求组装 | 稳定政策、动态事实、实际获准路由 | 动态运行信息不回填稳定前缀；自然语言不是执行证据 | 协议错误可恢复；普通解释不能因关键词丢弃 |
| 压缩快照 | 最后实际请求、会话、usage | 写读容量一致；凭据从当前授权路由解析 | 不匹配明确降级，不绕过身份验证 |
| 滚动摘要 | 已有摘要、剩余原文、实际窗口 | 每段按当前摘要重新核算输入；保留原始历史 | 摘要本身不可容纳时明确失败，不无效重复请求 |
| 媒体证据 | 授权读取的实际字节、工具调用 | 内容寻址不可变 blob；历史不重新解释可变源文件 | 缺失/篡改显式报告，不以新图片替换旧观察 |
| 结果投影 | 持久化 typed task result | 执行封存不等于成功；显示失败及未完成结果 | UI 不创造业务终态 |

## 实现账本

- [x] Core：规范调用身份贯穿准入和执行。
- [x] Core：删除截断恢复强制 append 的剩余分支。
- [x] Core：主链与 delegated 共用执行状态解释。
- [x] Host：移除自然语言“虚构执行”裁决。
- [x] Provider：稳定政策和动态运行消息保持边界。
- [x] 压缩：实际 fallback 路由恢复与容量一致性。
- [x] 压缩：滚动摘要动态装箱。
- [x] 媒体：统一不可变绑定及导出证据。
- [x] Mac：显示子任务真实业务结果。
- [x] 整体静态审计。
- [x] 整体测试及结果记录。

### 关键契约细化

Core 在模型 effect 中显式携带 `original_user_message_ids`，Host 使用私有来源标记区分真实用户消息与 user-role 运行事实，provider 发包前删除该标记。附件归属、Skill 决策范围和独立审查不再只看 role，既不通过文本关键字猜测，也不把运行数据当成用户新需求。Core checkpoint 升为 22，旧版本明确拒绝，不迁移或删除用户数据。

物理请求快照写读同为 32 MiB 存储边界（不是摘要输出上限）。快照写入在 ModelCompleted 交付之后；不能写入或没有物理见证时写 superseded 标记，防止重启误用上一轮快照。底层文件系统连失效标记也无法写入时显式诊断，不承诺不存在的持久性。恢复必须匹配当前获准路线和凭据身份，fallback 不借用主路线凭据。

媒体在工具结果发布前统一复制为内容寻址的不可变字节，所有操作类型在请求注入前校验摘要。归档失败保留真实执行回执，并另报媒体不可用，不把成功副作用改为失败诱发重做。`tool.progress` / `tool_media.prepared` 记录实际请求引用，不能据此声称模型已接收或理解。

Mac 子任务结果按 Answered、ActionCompleted、Incomplete、Blocked、Cancelled、Failed 展示文本、图标和详情，不再以生命周期 sealed 推断成功。显示不改变业务终态。

## 验收要求

先完成所有代码与测试用例，再交叉审计，最后统一运行测试。测试失败须保留首次失败原因，修复后复验；不得把旧二进制轨迹或旧测试计数计入本轮。

回归覆盖等价 JSON 参数身份、相关资源变化重新准入、NotStarted 不制造执行信用、截断无部分副作用、多语言普通解释不重采样、provider 前缀稳定性、fallback 重启恢复、快照容量对称性、滚动摘要超窗、媒体源覆盖与哈希不匹配、封存失败/未完成显示。

性能不预设未经测量的收益：记录测试时长、请求数量、快照字节、压缩请求次数；缓存改善须依赖实际 provider usage，不能以请求哈希代替。媒体归档仅证明归档字节和请求引用，不证明服务端模型理解或接收。

## 范围约束

不增加题目专用规则；不绕过权限、Store 来源验证或凭据身份检查；不为旧版本增加兼容逻辑；不清除用户会话或修改模型凭据。Mac 测试只使用 Test 产品目录，不覆盖开发者正在使用的签名 Debug 应用。

## 本轮统一验收（2026-10-13）

全部实现、交叉审计后统一运行。Rust 测试采用 `RUST_MIN_STACK=16777216`；通过数不重复计算 CLI 子进程内再次运行的 fixture。使用仓库 mbx 包装器；没有修改全局编译配置。

| 范围 | 通过 / 忽略 | 日志（本机 `/tmp/`） |
| --- | --- | --- |
| Core / Domain / Host protocol | 513 / 170 / 28，均无忽略 | `nola-chain-convergence-core-verified.log` |
| Application / Store | 315 / 409，各忽略 1 | 同上 |
| CLI library | 1797，忽略 4 | `nola-chain-convergence-cli-verified.log` |
| Core→Host integration | 145，无忽略 | `nola-chain-convergence-host-verified.log` |
| Application nonterminal cold replay | 7，无忽略 | `nola-chain-convergence-replay.log` |
| 导出 / Harbor / supervisor | 9 / 6 / 3 | `nola-chain-convergence-{export,harbor,supervisor}.log` |
| journal 完整正文与三个损坏负例（同一测试） | 1 | `nola-chain-convergence-journal-reader.log` |
| 新 CLI 中英文普通回答，精确正文、单次采样及 Answered seal | 2 | `nola-chain-convergence-process-verified.log` |
| 新 CLI SIGINT / SIGTERM / SIGKILL | 3 | `nola-chain-convergence-signals.log` |
| Mac Test 配置构建及任务投影（六种结果） | 4 | `nola-chain-convergence-mac.log` |

合计 Rust 3384 通过、6 忽略；Python 24 通过；Mac 4 通过。CLI 完整复验 118.45 秒，冷恢复集成 31.59 秒。`git diff --check` 通过。忽略项不是通过；大型 Rust 测试二进制仍有 Darwin `__eh_frame` 链接警告，本轮没有声称解决。

### 真实本地模型

以本轮新编译 Debug CLI、隔离项目/数据库/应用目录访问用户已有 `http://192.168.31.18:8081/v1` 路线，模型 `qwen3.8-flash-next-iq3_s`。冻结测试声明支持工具和推理，不修改用户配置。运行 `scripts/test_real_cli_multitool_recovery.sh`：首轮 2 次读取、2 份完整持久化工具结果；关闭进程后第二轮恢复同一会话，0 次工具调用，回答精确包含此前两个值；数据库会话、消息、parts 和 Core checkpoint 均有独立核验。

最终结果 `real_cli_multitool_recovery=passed`，日志 `/tmp/nola-chain-convergence-real-verified.log`。证据目录 `/var/folders/td/ly97ddt91k7gpfkvkqw0dzr80000gn/T/nola-cli-multitool-real.9igoUJ`，包含事件、provider 流量统计、数据库及持久化审计。该目录为本机临时证据，不是跨机永久归档。

Provider 共 5 次 HTTP 请求（含 2 次非生成探测），3 次生成均 HTTP 200，生成耗时约 4.35 / 5.93 / 3.80 秒。usage 报告总输入 53723 tokens，其中 cached 33039 tokens，加权读取比例约 61.50%。这是本次实测值，不是改造前后性能对比，也不能推广为 Anthropic 或压缩请求的命中率。

### 首次失败与修复记录

- Rust 编译先发现规范化后局部 tool_calls 绑定缺 mutable（`nola-chain-convergence-rust.log`），之后发现两个新增测试传入 `&str` 而 ID 构造要求 String（`nola-chain-convergence-rust-2.log`）；均已修复。
- CLI 两处、Host 两处旧测试仍要求动态故障或 Skill 记录与 System 政策混排，或缺少真实用户来源标记。改为严格断言固定政策、独立 User 事实、原 wire 前缀和私有字段删除，未降低生产安全要求。首次失败保留在 `nola-chain-convergence-rust-3.log` / `nola-chain-convergence-host.log`。
- 首次进程及真实模型脚本错误地从 JSON 通知查完整文本。当前协议只通知已提交 journal 边界；修正验收 reader，以 owner/序号/记录链/manifest 身份及长度核对后重建正文，不从 preview 取证。reader 是隔离验收一致性检查，不宣称密码学 seal 验证。保留 `nola-chain-convergence-process.log` / `nola-chain-convergence-real.log`，修正后使用新隔离运行再次通过。

### 结论边界

真实多工具和重启恢复已验证；本轮没有重新跑 Terminal-Bench 四题，不能声称任务得分提高。滚动摘要超窗、fallback 恢复、媒体篡改及协议前缀由确定性测试覆盖；本次短真实会话没有触发压缩、fallback 或真实视觉输入，因此不声称这些真实模型路径的延迟/成功率已测得。Mac 完成 Test 构建与投影测试，不等于全窗口人工视觉验收；按 apple-design 的可辨识结果原则使用文本、图标与详情区分，不能仅靠颜色表达业务状态。

后续用户要求的 Terminal-Bench 三题重跑已经结束：chain-v6 全部官方 0 分，三题均无所需交付物。终态持久化正常不等于解题能力得到改善；freight 反复写内部需求 JSON、music 解析与恢复失败、photonic 修改生成脚本后超时。完整对比及证据限制见 [LAN 三题重跑记录](../../../evals/benchmarks/terminal-bench/LAN-20261013.md)。该真实结果优先于对自动化契约测试通过的任何过度推断。
