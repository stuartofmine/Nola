> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# OpenCode、Codex 与 Nola：工具清单、能力差距与整改方向

日期：2026-10-17；2026-10-18收尾。状态：本轮整改与约定范围验收完成，逐项证据见实施台账；原始差距表不是当前未完成清单，验收仅限明确列出的范围，延期能力不视作已实现。

2026-10-18 用户更新验收口径：未跟踪变更范围不确定时，可以询问用户。最新三轮样本为已提交部分文档后持久化等待确认，不是整任务完成；不得继续用旧的“必须自动提交全部源码”作为唯一可接受行为，也不得将等待伪装为提交成功。现行进度与剩余审计见[实施台账](工具契约整改实施_20261017.md#当前状态唯一现行状态恢复工作先读此节)，下方矩阵保留历史验收边界。

### 阅读说明：基线与当前工作树

下文的 **198 个工具及接口缺口是首次审计基线**，不是当前工作树的全部状态。本次重新核对，当前 catalog 有 **200 个静态定义**：增加了 `terminal_input`、`language_service_list`、`language_service_query`，删除重复组合入口`implementation_brief`及其固定清单/专用风险启发式。当前变更尚未提交；真实验收范围见实施文档，不以工具数量代替能力验收。保留基线用于解释整改依据，不表示保留任何旧接口或兼容实现。

| 当前新增入口 | 已有代码契约 | 仍须验证或仍缺少的能力 |
|---|---|---|
| terminal_input | 对已有交互 bash 任务续输；绑定 task、attempt、worker generation 与 request_id；排队回执不等于已送达 | 真实 PTY 交互、取消、重启与未知送达状态；不能只凭接口存在判定与 Codex write_stdin 等价 |
| language_service_list | 列出配置的语言服务、项目范围、语言与配置指纹，不启动进程 | 配置在实际 Host 的接入及平台可用性 |
| language_service_query | 定义、引用、hover、文档符号、工作区符号、实现六类查询；启动配置的服务并校验文件快照 | 真实服务器联调；当前不含 OpenCode 的调用层级三类查询，也不含语义 rename；不保证整个工作区快照一致 |

当前实现源码：[工具注册](../../../cli/src/tools/parts/catalog.rs)、[终端入口](../../../cli/src/tools/parts/background.rs)、[语言服务入口](../../../cli/src/tools/language_service_tools.rs)。实施状态单独维护在[整改实施文档](工具契约整改实施_20261017.md)，不要把设计建议、代码落地和测试通过混成同一个状态。

### 当前整改对照（2026-10-17 集中验收后）

下表对照第 6 节原始清单，区分代码完成与证据。过程证据路径和仍在执行的验收以实施文档的唯一现行状态为准。

| 原始项 | 当前实现及已验证范围 | 必要收尾 / 未证实边界 |
|---|---|---|
| P0 目录与跨端协议 | Host 签发真实初始目录，提交摘要绑定 `tool_plan_ref`；Mac/iOS/Android/Web 不再按客户端 settings 推导远端能力。原 Run 目录供冷恢复读取；动态 MCP 权限使用当前 Core descriptor/revision，而非初始目录成员限制。Mac 定向 35 项全部通过；Android 新契约 3 项通过；Web 全量 400 项通过；iOS `RemoteHostToolPlanTests` 在 Test 模拟器实际执行 1 项通过、0 跳过，独立核实 `/tmp/nola-ios-host-plan-20261017.xcresult`。 | 这些契约测试不等于各平台真实远端 UI/生命周期全链路通过。设备桥继续独立身份/生命周期，不能混入 Host 初始目录。 |
| P0 编辑资源与结果 | 已收口补丁预像校验、资源闭包和单次执行；现有文件替换要求观察摘要，保留 no-op；patch 回归 25 项通过。 | 不能将组件通过解释成三轮真实模型提交任务通过；模型仍曾漏选用户要求的未跟踪源码，真实任务范围验收未完成。 |
| P0 权限复用 | 具体资源、执行身份与会话所有权统一判定，Host broker 与恢复组件已回归；普通读取与上传/本地修改分离。Mac谨慎模式真实192模型四轮验收通过，申请次数1/0/1/1；同资源复用、跨资源拒绝及跨会话隔离均核对。保留证据`/tmp/nola-mac-real-memory.kLyJkq/result.xcresult`，1通过0跳过。 | 证明该操作/作用域样本，不宣称所有工具或所有模型零重复申请；自动批准模式不是本样本依据。 |
| P0 长输出及压缩后引用 | 终端原文/摘要/唯一送达回执已真实验证。压缩04真实验收4967→936估算token，冷Run四个新结果完整重建8831字节且哈希一致，外owner读取被拒绝；独立证据`/tmp/nola-reference-compaction-20261017-04/cold-run-independent-verification.json`。 | 摘要提示与净缩预算冲突已修；维护Run不可用UI Part计数验收。冷读校验已排除旧历史页面，6项oracle通过；不保证任意模型每次压缩成功。 |
| P0 子代理生命周期 | 真实 Store/Core、独立进程回归包含冷权限恢复、owner 隔离、取消终态保存、不重启模型和 spawn 回执重开去重。192 本地模型样本 `nola-subagent-lifecycle-jg2m0iyo` 已完成三轮：真实 spawn/启动后取消、另一 owner 的 read/stop 明确拒绝、原 owner 在新 CLI 进程冷查询；canonical cancelled receipt 与唯一 attempt 保持不变，无完成副作用。 | 只证明已取消任务的冷查询不重派发，未测试运行中杀 Host 后恢复（`running_host_crash_recovery_tested=false`）。不将 MCP 杀 Host 恢复或这些 CLI 证据泛化为所有客户端 UI/恢复链路。 |
| P0 计划/目标、重复入口 | 删除无独立能力的 `implementation_brief` 及旧引用；保留明确不同语义的 `context_pack`/`focused_context`，选择语义测试通过。完成评估不再强制触发。 | 不以删除工具或测试数量证明计划、子任务与目标在真实跨进程执行中完全一致；优先复用上述生命周期验收，不新增第二套权威状态。 |
| P1 终端续输 | 真实 PTY/Unicode/长输出、唯一 delivered 回执，以及取消后另一 CLI 进程查询均通过；错误 owner/attempt/generation 与重复请求有组件覆盖。 | 运行中的进程崩溃、未知送达不重放仍须与已取消后的恢复查询明确区分；不笼统宣称所有重启场景等价 Codex。 |
| P1 语言语义 | 六类查询及同名不同绑定、跨文件定义/引用的真实模型验收 `nola-language-service-qo_re1to` 通过。显式 `supporting_documents` 绑定授权与有界快照，直接使用服务器 references，不拼接 definitions 冒充。 | 不保证全仓库索引完整；调用层级三类、语义 rename 未实现。已明确文本扫描/重命名边界，不将这些未实现能力算验收失败或默认为本轮扩展任务。 |
| 指标 | 已记录请求/发现耗时、schema 字节及估算 token、工具失败率和结构化重复拒绝；统计测试已通过。 | 缺失的真实计费 token、无效重试全分类保持未知，估算不能替代实测 p50/p95 与成本。 |
| 未知工具恢复 | 192模型单次工具名称故障注入后，Core拒绝、后续实际read_file成功、最终随机marker答案与正常完成均独立复核；证据`nola-unknown-tool-recovery-a3wl7687`。目录恢复债已与业务失败分离，Core全量517项通过。 | 属于明确披露的故障注入验收，不保证所有自然模型错误均能恢复；不能清除未知写入效果或业务验证义务。 |
| P2 编排/CSV 批量 | 已完成收益与现有 TaskSupervisor 复用边界评估。 | 有条件延期；本轮不为对齐工具数量扩张新编排系统。 |

收尾状态：权限、压缩复验、iOS协议测试及有界真实子代理三轮样本均已取得终态并独立复核。旧三轮Git自动全集提交样本保留失败；最新样本为部分提交后持久化等待确认，按用户更新口径可接受等待，但不是任务完成，不以单元测试代替真实提交结果。已有明确延期的P2不计作已实现。iOS测试脚本已改用Bash `pipefail`，防止xcbeautify成功吞掉xcodebuild失败；纯shell桩回归3项通过。提交验收脚本已统一当前终态契约，历史离线回归49项通过；最新待确认分支核验见实施台账。整体goal仍在审计收尾。

子代理样本首轮曾被 oracle 错误地拒绝可选 `requirements_update`，并非取消失败。修正为允许但不强制完成评估后，对原首轮事件、持久化终态和文件副作用重新核验，使用 `--resume-evidence` 在原隔离数据库继续第二、三轮；未重跑首轮、未覆盖失败证据。外部 oracle 同时要求当前 `nola.tool_result.v2` 中的精确 owner 拒绝，泛化传输失败不能冒充授权隔离通过。

## 1. 范围与结论

对比仓库内 `参考资料/opencode-dev`、`参考资料/codex-main` 与当前 Nola 工作树。两个参考目录不是独立 Git 仓库，因此不标注虚构的上游 commit，也不声称等同上游最新版。

Nola 基线 HEAD：`99b33e1fe8dbc2fb50ec1a4fc069f12a7ec02ea9`。本报告包含当前未提交的 Git 只读检查整改，不能把 HEAD 当成全部分析内容的唯一版本标识。

核心结论：

1. **Nola 不缺工具数量，缺的是部分能力深度和统一契约。** 静态 catalog 有 198 个命名定义；OpenCode 注册表有 17 类内建候选工具。数量不可直接比较质量，更不代表每轮模型看见的工具数量。
2. **交互终端续输由基线缺口进入实现验收阶段。** 基线只有命令、后台状态/等待/停止；当前工作树已增加 `terminal_input`。仍需核验实际 Host、模型发现、授权、送达回执和恢复全链路，不能只凭接口存在认定与 Codex `write_stdin` 等价。
3. **代码智能有“名称强于实际语义”的风险。** Nola 的导航走符号扫描，重命名走标识符匹配；不能当作语言服务器理解绑定关系后的导航与重构。OpenCode 的实验性 LSP 工具明确调用语言服务。
4. **权限体验需要执行层收口，而不是继续扩张命令白名单。** 本地 Git 检查、仓库写入、hook 副作用、网络上传是不同操作；不能用一次粗粒度 shell 授权统一解释，也不能为同一已授权操作重复弹窗。
5. **先修通发现→授权→执行→结果→恢复链路，再加新工具。** Nola 已有输出存档、检索、记忆、子代理、计划、浏览器和系统操作；“代码存在”不代表所有端、所有模型都可用。

本报告的“缺口”分三种：接口确实未发现、已有实现但语义较弱、已有能力但需要运行验收。不会把后两种说成完全没有。

## 2. 统计口径

| 层次 | 含义 | 不应混淆的事 |
|---|---|---|
| 静态定义 | 源码有 schema/handler | 不等于当前 Host 支持 |
| 可用目录 | 配置、平台、连接和模型能力筛选后的集合 | 不等于已经授权 |
| 当轮可见 | 直接暴露或搜索后提升的工具 | 不等于全部工具都进入 prompt |
| 执行授权 | 对具体资源与副作用作决定 | 不由工具名称、用户关键词或模型声明代替 |
| 内部操作 | model.stream、context.compact 等运行时能力 | 不算用户模型可直接调用的普通工具 |
| 动态扩展 | MCP、插件、设备桥、宿主扩展 | 无法用静态固定数字穷举 |

仅以实际注册路径为准。参考目录中存在但未注册的文件、测试工具、兼容 dispatch 入口，不计入默认工具数量。

## 3. OpenCode 工具清单

证据：[注册表](../../../参考资料/opencode-dev/packages/opencode/src/tool/registry.ts)。以下是候选全集，**不是同时启用的默认集合**。

| 工具 | 能力 | 暴露条件或边界 |
|---|---|---|
| invalid | 将无效工具调用变成可反馈错误 | 纠错入口，不是业务能力 |
| question | 向用户提出结构化问题 | 客户端类型/开关控制 |
| bash | shell 命令、工作目录、超时与输出 | AST 分析、资源检查和权限询问 |
| read | 文件/目录读取，按范围控制返回 | 具体媒体适配见实现，不能只按名字推断纯文本 |
| glob | 按文件模式找路径 | 与内容检索分工 |
| grep | 文件内容检索 | 与完整文件读取分工 |
| edit | 定位替换已有文件内容 | 与 apply_patch 按模型分流 |
| write | 写入文件 | 与 apply_patch 按模型分流 |
| apply_patch | 多文件补丁 | 当前规则为部分 GPT 型号启用，并排除 edit/write |
| task | 调用子代理完成任务 | task_id 支持后续接续；子代理有独立权限约束 |
| todowrite | 更新结构化任务清单 | 不是自动证明任务完成 |
| webfetch | 抓取指定网页内容 | 不等同浏览器交互 |
| websearch | 网络搜索 | opencode provider 或 Exa 开关 |
| codesearch | 外部代码资料/文档检索 | 不等于 Nola 本地 code_search |
| skill | 加载 Skill 内容 | 受可用 Skill 目录约束 |
| lsp | 语言服务导航与查询 | 实验开关；需要对应语言服务 |
| plan_exit | 请求从计划阶段进入执行 | 实验计划模式及 CLI 条件 |

LSP 的具体操作为：goToDefinition、findReferences、hover、documentSymbol、workspaceSymbol、goToImplementation、prepareCallHierarchy、incomingCalls、outgoingCalls。**其中没有 rename 操作**；不能把 OpenCode 的 LSP 查询能力夸大成已提供完整语义重构。[实现](../../../参考资料/opencode-dev/packages/opencode/src/tool/lsp.ts)

扩展入口：配置目录中的 tool/tools JS、TS 自定义工具，以及插件导出的工具。MCP 不应被误算为固定的第十八个内建工具；其数量取决于连接服务。

值得借鉴：

- 注册、模型筛选、描述生成、插件接入集中组织。
- 超长结果集中截断并保留完整输出引用；默认截断阈值为 2000 行/50 KiB，并可按配置调整，不是把所有原文反复送入模型。[输出处理](../../../参考资料/opencode-dev/packages/opencode/src/tool/truncate.ts)
- shell 分析得到具体模式和外部目录请求；“always”规则可解除同会话内匹配的其他待决请求。这里不推断规则已跨重启持久化。[命令](../../../参考资料/opencode-dev/packages/opencode/src/tool/bash.ts)、[权限](../../../参考资料/opencode-dev/packages/opencode/src/permission/index.ts)
- 工具简洁不是删掉实现细节，而是让执行器承担参数校验、权限与输出管理。

## 4. Codex 工具清单

证据：[工具计划与注册](../../../参考资料/codex-main/codex-rs/core/src/tools/spec_plan.rs)、[处理器目录](../../../参考资料/codex-main/codex-rs/core/src/tools/handlers)。

Codex 工具集合随模型、环境、功能开关、代理版本和宿主扩展而变化；没有一个适用于所有部署的固定总数。

| 分组 | 工具/入口 | 能力与条件 |
|---|---|---|
| 命令执行 | exec_command、write_stdin | UnifiedExec 路径：命令启动、运行会话、后续输入及输出读取 |
| 命令替代路径 | shell_command | 其他 shell 配置使用；不能与 UnifiedExec 简单相加为默认工具集 |
| 文件修改 | apply_patch | 按模型 patch 能力及执行环境配置 |
| 图像读取 | view_image | 执行环境中的本地图像检查 |
| 计划 | update_plan | 更新结构化计划状态 |
| 用户交互 | request_user_input | 功能/模式约束，不是所有模式必有 |
| 追加权限 | request_permissions | 功能开关及环境条件；权限有作用域和合并约束 |
| 延迟工具发现 | tool_search | 检索 deferred 工具元数据，后续暴露 schema；参考实现使用 BM25 |
| MCP 资源 | list_mcp_resources、list_mcp_resource_templates、read_mcp_resource | 有 MCP 服务时注册；资源发现不代替 MCP 工具发现 |
| 子代理 V2 | spawn_agent、send_message、followup_task、wait_agent、interrupt_agent、list_agents | 代理版本/功能启用；可以命名空间化 |
| 子代理另一配置路径 | spawn_agent、send_input、resume_agent、wait_agent、close_agent | 参考树仍有另一代理路径；Nola 不需要照搬双接口 |
| 批量代理作业 | spawn_agents_on_csv、report_agent_job_result | 作业开关；后者面向对应 worker |
| 程序化工具编排 | exec、wait | Code Mode 执行/等待；工具调用组合与输出筛选，不能和 shell exec_command 混淆 |
| 环境准备 | wait_for_environment | DeferredExecutor 条件 |
| 上下文预算 | new_context_window、get_context_remaining | TokenBudget 功能条件；不是所有模型都有 |
| 时间 | clock.curr_time、clock.sleep | 时间提醒与可选等待功能 |
| 插件安装建议 | request_plugin_install、list_available_plugins_to_install | Apps/Plugins/ToolSuggest 等条件；不是任意安装授权 |
| 网络搜索 | Provider hosted web search；宿主可扩展 web.run | cached/indexed/live 策略，区别供应商 hosted spec 与宿主函数入口 |
| 动态工具 | MCP 工具、DynamicToolSpec、Extension executor | 名称/schema 由配置与宿主贡献，不能硬列为固定内建全集 |

Code Mode 名称来自 [code-mode-protocol](../../../参考资料/codex-main/codex-rs/code-mode-protocol/src/lib.rs)，网络策略见 [hosted_spec](../../../参考资料/codex-main/codex-rs/core/src/tools/hosted_spec.rs)。

重要边界：

- 当前参考核心没有为 read_file、grep、list_dir 各自提供同名内建处理器，不代表不能读文件、检索、列目录；这些可通过 shell 完成。
- 图像生成、浏览器控制等宿主扩展不能因为当前聊天环境存在，就宣称是所有 Codex CLI 的固定能力。
- Skill 可以通过指令加载机制工作，不必存在名为 Skill 的函数工具。
- 测试专用 test_sync_tool、dispatch-only 的旧 shell 路径不作为建议 Nola 增加的功能。
- 不能从工具列表推导沙盒强度、并行正确性或恢复可靠性；需要执行层证据和测试。

## 5. Nola 工具清单与现状

权威静态定义：[catalog.rs](../../../cli/src/tools/parts/catalog.rs)。首次审计为 **198 个名称，198 个唯一值**，基线完整列表见附录；当前工作树为 **200 个定义**，新增三项并删除implementation_brief。动态 MCP/插件工具不计入这个静态数。

| 领域 | 代表工具 | 实现能力及注意点 |
|---|---|---|
| 文件与目录 | read_file、read_many_files、list_dir、glob、grep、find_files、find_up、file_tree | 文件、目录、内容检索；read_file 有媒体相关参数，不能说完全缺少图像/PDF入口 |
| 文件修改 | write_file、append_file、edit_file、edit_file_lines、multi_edit_file、apply_patch、move_file、delete_file | 多种粒度写入；需要共享版本校验、权限和执行回执 |
| Notebook | notebook_read、notebook_edit | 独立 notebook 工具面 |
| 代码理解 | repo_map、read_symbol、file_outline、code_search、workspace_symbols、symbol_search、implementation_search、code_navigation、call_graph | 本地代码索引/扫描面；名称不代表编译器级语义 |
| 重构 | replace_symbol_body、replace_in_files、rename_symbol | rename_symbol 当前使用符号匹配扫描，不是 LSP rename |
| 诊断与验证 | code_diagnostics、failure_context、failure_triage、test_context、validation_plan、verification_review | 诊断/计划/回顾入口；code_diagnostics 当前扫描不等于执行编译器或证明测试通过 |
| 上下文构造 | context_pack、focused_context、impact_analysis、instruction_context 等 | 已删除implementation_brief重复组合与固定清单；保留有明确输入/输出区别的探索和上下文工具 |
| Git/工作区 | workspace_status、workspace_access、git_diff、git_history、git_blame、git_show、git_commit、workspace_checkpoint | 已有专门 Git 能力；不需要以“新增 git 工具”解决重复授权 |
| shell/后台 | bash、bash_command_review、bash_background_status、bash_background_wait、bash_background_stop、terminal_input | 后台管理及新增交互输入；输入绑定任务、执行尝试与工作进程代际，排队不等于已送达 |
| 语言服务 | language_service_list、language_service_query | 新增配置发现及六类语义查询；无自动安装、无调用层级查询、无语义 rename，不以扫描冒充语言服务 |
| 发现/原文 | tool_search、tool_output_list/read/search/find | 按需发现和完整输出召回已有入口 |
| Skill/SOP | Skill、skill_update、skill_skip、SOP、sop_run_update、nola_orchestrate | 加载与流程状态；内容不能自行授予执行权限 |
| 记忆/历史 | memory_save、memory_search、document_search、conversation_recall | 已有记忆写入/检索与会话原文召回；故障应查可用目录和路由，不应认定缺工具 |
| 子代理/目标/计划 | spawn_agent、list_tasks、read_task、wait_task、send_agent_message、stop_task、update_agent_todos、agent_task_status、goal_complete、propose_agent_plan、agent_plan_file | 生命周期与目标入口较丰富；需要统一 owner、状态和结果协议 |
| 用户/配置 | request_user_input、set_thinking_mode、environment_probe、system_time | 人机交互、思考档位、环境与时间 |
| 网络 | web_search、web_fetch | 搜索/抓取与浏览器操作应保持分离 |
| 托管浏览器 | browser_* | 生命周期、导航、快照、点击、输入、页面管理 |
| Chrome 原生桥 | chrome_native_* | DOM/可访问性/存储/控制台/网络事件与交互 |
| 设备操作 | computer_* | 截图、OCR/文本、可访问性、键鼠、剪贴板、应用/窗口/菜单与断言等待 |
| 日历提醒 | search/create_calendar_event 等 | 平台服务操作；须按 Host 能力及系统许可提供 |

### 5.1 当轮暴露不是 198 个

[model_conversion.rs](../../../cli/src/agent_host_driver_parts/model_conversion.rs) 明确按档位设置 Visible/Deferred：

- 简单：6 个候选直接入口，tool_search、request_user_input、Skill、read_file、workspace_access、workspace_status。
- 中等：INITIAL_MODEL_TOOL_IDS 中 19 个候选直接入口。
- 复杂：在中等集合上增补目录/检索/读写/网络入口，去重后 25 个候选直接入口。

这是**筛选前的内建候选数**，不是实际请求抓包数。Host 可用性、动态工具、运行中提升会改变最终集合；可见性改变不授予权限。不能据 198 个静态定义断言每轮 prompt 注入了全部工具。

### 5.2 动态工具与内部能力

插件、MCP、设备桥还会贡献工具；mcp_connect 及 MCP 资源/模板/Prompt 接入不包含在上述静态计数内。应以 Host 的当前目录和连接代际为准。[MCP 接线](../../../cli/src/agent_host_driver_parts/mcp_connection.rs)

model.stream、context.compact 等内部 capability 不作为一般业务工具计数。对话压缩、目标恢复与工具输出召回也不应混成同一接口。

### 5.3 两个需要避免的错误宣传

- code_navigation 当前调用 scan_symbol_matches / scan_implementation_matches；code_diagnostics 调用 scan_code_diagnostics。[实现](../../../cli/src/tools/parts/context.rs)
- rename_symbol 使用 symbol_reference_regex 并遍历文件；同名文本可能不是同一语义绑定。[实现](../../../cli/src/tools/parts/checkpoints_review.rs)

这些实现可作为快速索引和辅助线索，但不能承诺跨语言语义正确性。建议结果明确标识来源和置信边界。

## 6. 能力差距矩阵

| 维度 | OpenCode | Codex | Nola 判断与优先级 |
|---|---|---|---|
| 普通文件操作 | 小而直接的读/改/写/patch | shell + patch | 已有且更细；优先统一，不再补同类名字 |
| 多文件修改一致性 | patch/edit 执行器 | patch 执行器 | 核验多入口是否共享版本、资源与回执，P0 |
| 交互式终端 | 注册 bash 不是通用续输接口 | exec_command + write_stdin | 已新增 terminal_input；尚需全链路验收，P1 |
| 代码语义导航 | 可选 LSP 九类查询 | 核心工具不承诺专用 LSP | 扫描入口与新增六类 LSP 查询并存；仍缺调用层级三类查询，P1 |
| 安全重命名 | 此 LSP 工具不含 rename | 可通过编辑实现，不保证语义 | Nola 名称容易过度承诺；先标边界，再补语义后端 |
| 动态发现 | 模型/代理筛选、插件/MCP | deferred + tool_search | Nola 已有，需测发现成功与停用后的拒绝，P0 |
| 长输出 | 统一截断 + 完整结果文件 | 执行输出预算/会话管理 | Nola 已有原文库和搜索；测持久化、引用与压缩后的可达性，P0 |
| 程序化编排 | 自定义工具/代理组合 | Code Mode exec/wait | 未在 Nola 静态目录发现等价执行入口；先评估收益，P2 |
| 批量代理作业 | task 通用派发 | CSV 作业及结果回报 | Nola 有子任务，不等于批量结构化作业协议；按需求补，P2 |
| 子代理生命周期 | task_id 接续 | 两类配置化代理工具族 | Nola 已有多入口，重点所有权/完成/取消一致性，P0 |
| 计划/目标 | todowrite、plan_exit | update_plan | Nola 更丰富，但需一个权威状态源，P0 |
| 网络/浏览器 | webfetch/search；插件可扩展 | hosted search/宿主扩展 | Nola 原生浏览器广；普通检索不应被强制升级到浏览器 |
| 桌面/日历/记忆 | 常借扩展 | 常借宿主/扩展 | Nola 静态能力更广，但平台可用性不能泛化 |
| 权限复用 | 具体模式规则，解决匹配待决 | 沙盒、exec policy、作用域权限 | Nola Git 检查已窄化；提交链路与会话内复用仍需独立验收，P0 |
| 平台一致性 | 各客户端呈现共享执行状态 | 宿主定义执行与 UI 边界 | Nola 不能以 Rust catalog 存在证明 Mac/iOS/Android/Web 都接通，P0 |

P0/P1/P2 是本报告建议顺序，不是新建执行任务，也不是已完成状态。

## 7. Git 重复权限：应学什么，而非照搬什么

OpenCode 不是靠一个专用 git_commit 工具解决全部问题，而是通过命令分析、目录范围和可复用规则组织权限。Codex 将命令执行、沙盒限制、策略匹配与追加权限分层；也不能推断任何 git commit 必然免确认。

Nola 当前新增的 Git read plan 只解决部分确定安全的元数据命令与简单管道，不是完整 shell 解释器，更不是仓库永久写入授权。已有实现见 [Git 检查权限设计](../../软件架构设计/模块/07-权限与平台/GIT_INSPECTION_AUTHORITY.md)、[解析与执行计划](../../../cli/src/tools/git_read_plan.rs)。

建议统一为：

1. 工具把具体操作解析为类型化资源/副作用计划；模型不得自报“只读”替代解析。
2. Core 依据会话配置和已授权作用域作决定；客户端仅展示并提交决定。
3. 执行器消费同一个冻结计划，防止审批命令与真正执行命令不一致。
4. 同会话同范围授权按明确期限复用；资源、命令结构或副作用变化必须重新判断。
5. Git 仓库本地写入与远端上传分离；hook 若可能执行额外命令，应纳入真实执行边界，不能假装不存在。
6. 每次拒绝/再次申请给出机器可读原因。UI 不应把版本不匹配、平台限制、连接中断统称“需要权限”。

“全部允许”应按产品当前契约处理应用层审批，不伪造系统权限、不忽略执行失败，也不能让安全边界判断散落在每个客户端。

## 8. 建议模块契约与验收

以下均为建议设计；不能据表声称这些抽象类已经全部存在。

| 模块/所有者 | 输入 | 输出 | 必须封装的责任 |
|---|---|---|---|
| ToolCatalog / Host | 平台可用性、插件/MCP注册、档位 | 版本化 CapabilityDescriptor | 名称、schema、来源、能力边界和发现元数据 |
| ToolDiscovery / Core | 模型检索、当前目录快照 | 有效工具引用及提升集合 | 不按用户自然语言关键词选工具，不授予权限 |
| PermissionAuthority / Core | 具体操作计划、会话配置、授权记录 | allow/ask/deny 及理由/作用域 | 一处决策、会话所有权、授权复用 |
| ToolExecutor / Host | 已准入的冻结计划、执行身份 | 状态事件、结果或未知状态 | 不重新解释权限；幂等、取消和恢复 |
| ToolResultStore / Store | 原始输出、执行身份 | 持久化引用、受限摘要、检索结果 | 原文唯一保存、权限一致、压缩后可回查 |
| CodeIntelligence / 后端 | 项目、文件版本、位置、操作 | 带来源的符号/诊断/编辑计划 | 区分文本扫描与语言语义 |
| TerminalSession / 执行宿主 | 命令、PTY选项、后续输入 | 会话ID、增量输出、退出状态 | 所有权、背压、超时、取消；不能跨会话续输 |
| TaskSupervisor / Core | 派发计划、配置快照、预算 | 子任务状态与结构化结果 | 继承、隔离、终态和目标关联 |
| 客户端展示层 | 以上规范事件与回执 | 工具卡片、授权控件、结果视图 | 不制造第二份执行真相、不重复授权 |

建议验收指标（**目标，不是实测结果**）：

- 同一个有效授权覆盖的重复动作：重复弹窗 0 次；超范围动作必须被拦截或申请。
- 相同工具调用在各客户端进入同一 Core 决策；客户端差异只来自真实 Host 能力。
- 冻结目录内可用工具检索成功；停用、代际变化、未知工具有明确可恢复错误，不偷偷改用 bash。
- 超长输出完整原文只存一次；摘要明确截断和引用，重启/压缩后仍可读取。
- 记录每轮工具 schema token、发现耗时、调用失败率、无效重试次数、输出读取量；报告 p50/p95，不用工具数量代替性能。
- 真实终端样本覆盖等待输入、长输出、取消、重启后的状态查询；未知结果不自动重放写操作。
- 代码导航覆盖同名不同绑定、跨文件引用和不支持的语言；文本扫描必须披露限制。
- 中英追问、模糊词、未知工具和自然延续均进入真实模型测试；不为测试题硬编码路由。

## 9. 推荐推进次序

### 已落地的收敛决定

- 删除`implementation_brief`：它重复组合候选文件、指令、测试/诊断建议和固定清单，没有独立执行能力；已同时删除专用启发式、目录、分发和各端旧引用，不保留别名。
- 保留`context_pack`与`focused_context`：前者按显式文件顺序及可选Git变更形成无query项目快照，后者按query排名并返回匹配原文。种子路径是加权不是扫描边界；已明确参数说明并增加选择语义对照测试。不能为压低工具数丢掉不同的输入/输出能力。
- Code Mode与CSV批任务的本轮收益评估已形成[编排决策](编排工具收口与批量能力决策_20261017.md)：不为工具数量对齐新建第二调度系统；保留明确触发条件、既有TaskSupervisor复用边界和量化验证方式。此为有条件延期，不声称已具备等价能力。

1. **P0：收口契约。** 先核实各注册入口的 schema、权限计划、执行回执、结果存储和 UI 投影一致；优先解决真实会话里失败的动作。
2. **P0：减少重复表面。** 审核 context_pack/focused_context/implementation_brief 等重叠入口及多种编辑接口；按真实使用收益保留，内部服务可以复用但不必全部成为模型工具。
3. **P1：补执行深度。** 交互终端会话、语言服务后端；明确语义能力边界。
4. **P2：按收益加编排。** Code Mode、CSV 批任务应在稳定的权限和执行协议上实现，不先建设第二套执行系统。
5. 不复制参考项目的历史兼容路径，不增加旧协议别名，不迁移或改写实际用户数据。

## 10. 证据与复核边界

主要源码入口已在各章节链接。另见：

- [Nola 工具分发](../../../cli/src/tools/parts/approval.rs)
- [Nola 权限解析](../../../cli/src/tools/permission_resolution.rs)
- [Nola 输出存储](../../../cli/src/tools/output_store.rs)
- [Nola 任务执行](../../../cli/src/task_runtime_parts/execution.rs)
- [Codex 工具发现 spec](../../../参考资料/codex-main/codex-rs/core/src/tools/handlers/tool_search_spec.rs)
- [OpenCode 子任务](../../../参考资料/opencode-dev/packages/opencode/src/tool/task.ts)

检索过 OpenAI 官方 CLI/安全文档入口作为资料核对；本报告具体工具名和条件以以上**本地源码**为依据，不以当前在线文档反推参考快照。

审计时三个关键文件 SHA-256：

| 文件 | SHA-256 |
|---|---|
| OpenCode tool/registry.ts | cc1d806f0cfaf206bda3b7962f4a1f79c82c2677b19e0c5477af15433ea86696 |
| Codex tools/spec_plan.rs | d7e53afa65787f84e1df5ab4fcdb48a32b33ed1adbd7fa835e73406690dd0ede |
| Nola tools/parts/catalog.rs | 0d697835d0ff6ee126328f960435aaf6567c5a7978389b9dd884a07233cbc696 |

本次只做源码与文档核对，没有重跑真实模型、浏览器或原生应用，因此不宣称全部工具可运行，也不报告未经测量的性能优劣。

## 附录：Nola 基线静态工具完整列表（198 个）

按 catalog 源码顺序，保留精确大小写；不包含动态注册与内部 capability。此清单用于完整性复核，不意味着建议把全部 schema 同时发送模型。

当前工作树完整集合为以下基线列表，加阅读说明中的三个新增工具、减已删除的implementation_brief，共200个。此次文档复核的catalog SHA-256为`3797a8242e531e1ea540716c6e5584d694c86ff8f6c6252c61b43374979c26b3`（含语言查询显式supporting_documents schema，尚待集成验收）；上文证据表保留首次审计指纹。基线列表中的已删除名称仅作历史对比，不可调用。

```text
environment_probe
read_file
read_many_files
notebook_read
notebook_edit
file_outline
read_symbol
repo_map
code_search
workspace_symbols
symbol_search
implementation_search
code_navigation
call_graph
code_diagnostics
failure_context
failure_triage
file_dependencies
coding_conventions
project_overview
instruction_context
Skill
skill_update
SOP
sop_run_update
skill_skip
nola_orchestrate
agent_context
command_context
agent_config_context
ci_context
context_pack
focused_context
implementation_brief
impact_analysis
validation_plan
test_context
format_plan
format_files
related_files
write_file
append_file
move_file
delete_file
replace_symbol_body
edit_file
edit_file_lines
multi_edit_file
replace_in_files
rename_symbol
apply_patch
workspace_status
workspace_access
change_review
diff_review
verification_review
handoff_context
git_diff
workspace_checkpoint
git_history
git_blame
git_show
list_dir
changed_symbols
file_tree
find_files
find_up
glob
grep
tool_search
tool_output_list
tool_output_read
tool_output_search
tool_output_find
set_thinking_mode
memory_save
memory_search
document_search
spawn_agent
list_tasks
read_task
wait_task
send_agent_message
stop_task
update_agent_todos
request_user_input
agent_task_status
goal_complete
propose_agent_plan
agent_plan_file
bash_command_review
bash
git_commit
bash_background_status
bash_background_wait
bash_background_stop
system_time
web_fetch
web_search
browser_status
browser_start
browser_stop
browser_snapshot
browser_navigate
browser_click
browser_type_text
browser_screenshot
browser_tabs
browser_switch_page
chrome_native_status
chrome_native_inspect
chrome_native_screenshot
chrome_native_dom_snapshot
chrome_native_accessibility_snapshot
chrome_native_storage_state
chrome_native_console_logs
chrome_native_network_events
chrome_native_list_pages
chrome_native_switch_page
chrome_native_open_page
chrome_native_hover
chrome_native_click
chrome_native_drag
chrome_native_type_text
chrome_native_select_option
chrome_native_set_checked
chrome_native_press_key
chrome_native_scroll
chrome_native_assert
chrome_native_wait_for
search_calendar_events
create_calendar_event
search_reminders
create_reminder
update_reminder
conversation_recall
computer_status
computer_observe
computer_list_displays
computer_screenshot
computer_screenshot_element
computer_screenshot_window
computer_read_text
computer_find_text
computer_move_to_text
computer_click_text
computer_select_text
computer_assert_text
computer_wait_for_text
computer_wait
computer_accessibility_snapshot
computer_element_at
computer_press_at
computer_type_at
computer_scroll_at
computer_move_to_element
computer_click_element
computer_press_element
computer_focus_element
computer_perform_element_action
computer_type_element
computer_set_element_value
computer_assert_element
computer_wait_for_element
computer_get_position
computer_move_mouse
computer_mouse_down
computer_mouse_up
computer_click
computer_drag
computer_drag_element
computer_type_text
computer_get_clipboard
computer_set_clipboard
computer_copy_selection
computer_assert_clipboard
computer_wait_for_clipboard
computer_list_apps
computer_assert_app
computer_wait_for_app
computer_list_windows
computer_focus_window
computer_set_window_bounds
computer_perform_window_action
computer_close_window
computer_assert_window
computer_wait_for_window
computer_list_menus
computer_select_menu
computer_activate_app
computer_quit_app
computer_open_item
computer_press_key
computer_hotkey
computer_key_down
computer_key_up
computer_scroll
computer_scroll_element
```

