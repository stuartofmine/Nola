# Nola · 业务原型与交互设计

更新：2026-11-20。桌面左侧统一为「对话 / 专家与技能 / 自动化中心 / 记忆与数据源」四项固定导航，已安装插件提供的工作页在同一侧栏按名称出现；常驻对话搜索与列表 + 底部单一设置窗口入口；移除顶栏双侧栏图标与浏览器式内容标签。右侧「任务 / 产物 / 上下文」采用无外层手风琴的三个独立扁平滚动区，其中「上下文」提供原始消息与摘要计数、可各自展开的知识点/会话 Wiki/会话回忆，托管浏览器仅在未连接或异常时提示。桌面输入栏精简为「模型/外部代理/远端主机」「选择文件夹（临时工作区）」「@ 专家」三项控制。当前处理中显示单色流光，尊重减少动态效果。

新增覆盖当前业务清单的可操作场景原型：**71 个场景、六个平台视角**，各场景的状态数见[生成清单](docs/BUSINESS-SCENARIOS.md)。既有完整交互原型保留；新入口补齐首次引导、账户授权、模型配置、宿主机、权限、Goal、长任务恢复、MCP/Skill、资料记忆、协作、计划通知及维护流程的正常、异常和恢复状态。

## 连接宿主机

[共享这台 Mac](scenarios.html?scene=share-host&platform=mac) → [手机配对](scenarios.html?scene=host-pair&platform=ios) → [Android 模型与管理入口](scenarios.html?scene=host-pair&state=2&platform=android)。Mac 连接另一台服务器使用[远端连接表单](scenarios.html?scene=host-pair&platform=mac)。从桌面原型的设置 → 连接，或手机原型的模型页，也能进入对应流程。

「下次演示回执」可控制下一次验证结果。试着在验证中取消、在错误表单中改令牌，以及先配对再修改地址，核对原连接是否保留。完整的平台入口、状态与原型图见[连接宿主机](../shared/HOST_CONNECTION_WORKFLOW.md)。本地浏览器回放使用 `PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_host_connection.cjs`。

## 打开原型

```sh
python3 -m http.server 8071 --bind 127.0.0.1 --directory docs/design/prototype
```

- [桌面侧栏中的任务看板](scenarios.html?scene=harness-plugins&platform=mac&state=page)演示直接在主窗口切换对话、插件管理和包内页面。
- [运行时插件](scenarios.html?scene=harness-plugins&platform=ios)进入后显示已安装插件；添加时选择运行时，安装后从同一行直接打开随包页面。
- GitHub Skill 安装支持按名称查找、直接使用目录链接、审阅安装位置、拒绝或单次允许、在新会话读取随包文件，以及在设置中查看固定来源和停用。手机可分别预览[本机安装](scenarios.html?scene=github-skill&platform=ios)和[配对 Host 安装](scenarios.html?scene=github-skill-host&platform=android)。桌面与终端可在同一场景切换平台。[桌面审阅](scenarios.html?scene=github-skill&platform=mac&state=review)
- [过程与来源预览](reply-preview.html)：切换桌面/手机及明暗，检查正文/表格中的灰色小号内联引用、步骤间等待、展开中的运行标题、未完成状态语义与 31 条引用的展开收起；完整原型的“本地模型，怎么选更合适”会话也使用同一组件。
- [移动端预览](mobile.html)：直接在 Mac 浏览器审阅手机界面，可切换 iOS/Android、明暗和 320/390/430px；外观切换保留草稿。
- [持续流光预览](index.html?stream=hold)：发送一条演示消息后保持处理中，便于审阅流光和减少动态效果；点击停止结束演示，不调用模型。
- [全业务场景原型](scenarios.html)：本地服务地址 `http://127.0.0.1:8071/scenarios.html`。按业务、平台、主题和状态审阅，可直接操作表单、分支、确认与取消；场景可通过 URL 分享到具体状态。
- [完整交互原型](index.html)：本地服务首页。保留实际会话列表、全屏聊天、模型档位、编辑重发、独立检查器、资料和通用设置；设置中的“原型预览”可进入全业务场景。
- [业务覆盖与维护规则](docs/BUSINESS-SCENARIOS.md)：逐项入口、状态数、适用平台和当前 Web 15 路由映射。
- [覆盖 CSV](FEATURE-COVERAGE.csv) / [结构化场景清单](BUSINESS-SCENARIOS.json)：71 项完整清单；旧覆盖表不再把过时的 Today 导航或历史测试当现行结论。
- [验证与边界](docs/VALIDATION.md)：区分本轮合成原型回放、历史记录、原生客户端和真实模型证据。

启动上述本地服务后，运行 `node docs/design/prototype/verify_github_skill.cjs` 可回放安装流程并导出截图。脚本使用项目配套的 Browser Use 运行时，也可通过 `NOLA_BROWSER_USE_PYTHON` 指向已安装 `apps/web/browser_use_runtime/requirements.lock` 的 Python 环境。

## 现行设计合同

现行产品行为以[对话交互](../shared/CONVERSATION_UI_DESIGN.md)、[会话检查器](../shared/CONVERSATION_INSPECTOR.md)、[视觉系统](../shared/DESIGN_SYSTEM.md)和[宿主机流程](../shared/HOST_CONNECTION_WORKFLOW.md)为准。原型交互与视觉参数见 [UI-INTERACTION.md](docs/UI-INTERACTION.md)、[DESIGN-SPEC.md](docs/DESIGN-SPEC.md)；[客户端差异](docs/IMPLEMENTATION-COMPARISON.md)继续记录原型与生产之间的距离。

手机仍以对话列表进入聊天，主导航为对话、活动、资料、设置；桌面采用统一单侧栏、简洁 48px 单顶栏（侧栏开关/新对话 + 对话或分区标题 + 右侧搜索与三区检查器开关）与独立 820×620 设置窗口。检查器「任务 / 产物 / 上下文」为三个直接滚动区域（无外层折叠手风琴），知识点与会话 Wiki 均支持原地展开编辑；运行标题跟随整轮状态，来源使用默认三条的小号灰色编号列表，全界面零 SHA-256 哈希与字节偏移展示。没有蓝色常驻选中边框，复制没有常驻边框与底色；通用集中外观、通知、软件更新与启动。

## 演示边界

场景预览不调用模型、Host、OAuth、系统权限、文件、调度或 Git，不下载更新、不接受真实凭据。适用平台表示设计覆盖范围，不是生产能力或原生验收声明；CLI 预览是文本布局草图，不增加命令。外部旧 Site 未随本轮更新，不再作为当前资产入口。

新场景原型仅在本页内存保留各平台/场景的草稿与状态，刷新重置。既有完整原型的主题与减少动态效果可使用浏览器存储；其他样例在刷新后重置，没有账号或跨设备同步。旧脚本 `verify.py`、`verify_commands.py` 属于历史记录，不作为本轮通过依据。

## 检查入口

```sh
python3 docs/design/prototype/sync_business_scenarios.py --check
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_business_scenarios.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_business_layout.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_page_sync.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_sidebar_sync.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_mobile_ui.cjs
# 需要已构建的 Browser Use runtime（默认 apps/web/playwright_runtime/browser_use）
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_reply_presentation.cjs
```

浏览器回放需要本机 Chrome 与已有 Playwright runtime 依赖；缺少依赖时运行 `npm ci --prefix apps/web/playwright_runtime`。新增的过程/来源脚本还需要 Browser Use 本地执行 runtime；仅安装 Node 依赖不足以运行它们。有 `uv` 时可构建默认目录：

```sh
./scripts/bundle_browser_use_runtime.sh apps/web/playwright_runtime/browser_use
# 若使用其他构建目录，运行检查时设置 NOLA_BROWSER_USE_ROOT=<相对仓库的-runtime-目录>
NOLA_BROWSER_USE_ROOT=apps/web/playwright_runtime/browser_use node apps/web/playwright_runtime/reference_presentation.cjs
```

此构建准备固定依赖和 Python，不申请模型凭据或启动模型请求。执行与观察的职责见[浏览器架构](../../architecture/模块/06-工具与证据/BROWSER_AUTOMATION.md)。

生成清单使用 `sync_business_scenarios.py`，会检查状态分支、源码文件及生产 Web 路由新增遗漏。浏览器脚本阻止外部请求，回放、截图图册和回执默认写入仓库下被忽略的 `.artifacts/`，不进入源码仓库。原型测试通过不代替真实客户端验收。

## 专家角色、档位与 @ 引入

完整原型增加 `expert-roles.js` 的独立扩展：通过聊天中的“专家 → 专家库”或手机（iOS 与 Android）“设置 / `+` 面板 → 管理专家”配置。七个角色与 Core 中央目录保持同一份静态内容快照，可先展开提示词、工具组合及配套 Skill，再明确应用；替换当前自定义模板需确认，模板库内容保留。主会话仅由 `Nola` 面向用户发言，不提供专家气泡配色入口。

专家推理档位来自合成模型声明：五档模型、含 Max 的四档模型，以及未声明能力的模型。继承默认模型时读取该演示默认档案；自动表示继承，不预选 none。选择角色不授权工具，也不自动激活 Skill。

在[专家提及示例](index.html?demo=expert-mention)或[手机预览](mobile.html)打开「让 Nola 更像 Nola」，点首条消息中的「@研究」，从资料进入只读专家会话。回到主会话后可在输入框键入「请 @代码」试选人。对照[桌面输入与手机已发消息实图](../shared/EXPERT_MENTIONS.md)可逐步核对。

要看旧配置，先在专家选择中将「研究」以最新配置重新引入，再点首条消息的原名字。资料会标为「发送时的配置 · 历史只读」。编辑重发首条消息时，可以取消并保留整段对话，也可以将旧提及转为普通文字后提交，观察同一会话以及专家任务历史一起回溯。

本地回放提供两条可对照的路径。选择「研究」发送一条新消息，Nola 直接回复；选择「代码助手」发送则回放一项代码审查任务。两者的已发名字均可打开资料。实际客户端由主 Agent 按请求决定是否派发。

输入 `@代码` 可从完整演示目录加入尚未参与的代码专家，并在原句插入相应名字。目录中的“研究”专家特意比已有会话快照新一个版本：输入 `@研究` 可沿用现有快照；专家列表中可选择“以最新配置重新引入”并审阅更改，确认后旧上下文进入保留记录，已有消息不改写。新空白会话选中专家只记录选择，发送演示消息时才建立合成成员上下文。

以下验证入口已执行：静态合同通过；Browser Use 在 1440px / 390px 与明暗两主题的 4 组组合全部通过。既有 Inspector、草稿保留与移动导航的 10 组回归也通过。原型截图已导出并实际查看；这不计作真实模型或原生客户端通过：

```sh
node docs/design/prototype/verify_expert_roles.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_expert_roles_browser.cjs
```

第二个脚本只允许本地原型地址，浏览器操作由项目固定的 Browser Use runtime 执行，禁止非原型 origin 请求；覆盖桌面/手机、明暗、声明档位、角色替换确认、ID 提及、重新引入和历史保留，并导出截图。已有 Inspector、移动端、来源展示与页面同步脚本继续保留；这些是合成原型验证，不是真实 Host、真实模型或客户端验收。

专家协作在主会话 `Nola` 回复中以内联过程摘要卡片呈现，并在桌面右侧「任务 > 专家会话」与移动「会话详情」中挂载只读专家会话入口（不在左侧或移动会话列表中混排子会话树）；点击打开只读阅读器。当前子会话可回父会话选中专家继续；重新引入前的旧上下文保留快照，不提供发送或静默升级。`expert-history.js` 仅用内存合成记录。设计约束见 [多智能体导航契约](../shared/MULTI_AGENT_NAVIGATION.md) 与 [专家子会话与历史阅读](../shared/EXPERT_CONVERSATION_HISTORY.md)。
