# Nola · 业务原型与交互设计

更新：2026-11-12。桌面左侧仅保留对话、资源两个入口；右侧概览、计划、目标、执行、上下文各有独立职责，知识编辑器与历史记录默认收起。当前处理中显示单色流光，尊重减少动态效果。

新增覆盖当前业务清单的可操作场景原型：**68 个场景、220 个状态、六个平台视角**。既有完整交互原型保留；新入口补齐首次引导、账户授权、模型配置、宿主机、权限、Goal、长任务恢复、MCP/Skill、资料记忆、协作、计划通知及维护流程的正常、异常和恢复状态。

## 打开原型

```sh
python3 -m http.server 8071 --bind 127.0.0.1 --directory docs/design/prototype
```

- [过程与来源预览](reply-preview.html)：切换桌面/手机及明暗，检查正文/表格中的灰色小号内联引用、步骤间等待、展开中的运行标题、未完成状态语义与 31 条引用的展开收起；完整原型的“本地模型，怎么选更合适”会话也使用同一组件。
- [移动端预览](mobile.html)：直接在 Mac 浏览器审阅手机界面，可切换 iOS/Android、明暗和 320/390/430px；外观切换保留草稿。
- [持续流光预览](index.html?stream=hold)：发送一条演示消息后保持处理中，便于审阅流光和减少动态效果；点击停止结束演示，不调用模型。
- [全业务场景原型](scenarios.html)：本地服务地址 `http://127.0.0.1:8071/scenarios.html`。按业务、平台、主题和状态审阅，可直接操作表单、分支、确认与取消；场景可通过 URL 分享到具体状态。
- [完整交互原型](index.html)：本地服务首页。保留实际会话列表、全屏聊天、模型档位、编辑重发、独立检查器、资料和通用设置；设置中的“原型预览”可进入全业务场景。
- [业务覆盖与维护规则](docs/BUSINESS-SCENARIOS.md)：逐项入口、状态数、适用平台和当前 Web 14 路由映射。
- [覆盖 CSV](FEATURE-COVERAGE.csv) / [结构化场景清单](BUSINESS-SCENARIOS.json)：68 项完整清单；旧覆盖表不再把过时的 Today 导航或历史测试当现行结论。
- [验证与边界](docs/VALIDATION.md)：区分本轮合成原型回放、历史记录、原生客户端和真实模型证据。

## 现行设计合同

现行产品行为以[对话交互](../shared/CONVERSATION_UI_DESIGN.md)、[会话检查器](../shared/CONVERSATION_INSPECTOR.md)、[视觉系统](../shared/DESIGN_SYSTEM.md)和[宿主机流程](../shared/HOST_CONNECTION_WORKFLOW.md)为准。原型交互与视觉参数见 [UI-INTERACTION.md](docs/UI-INTERACTION.md)、[DESIGN-SPEC.md](docs/DESIGN-SPEC.md)；[客户端差异](docs/IMPLEMENTATION-COMPARISON.md)继续记录原型与生产之间的距离。

手机仍以对话列表进入聊天，主导航为对话、活动、资料、设置；桌面是单顶栏、内容标签与独立侧栏。检查器区段原地展开，模板归入模型配置，推理按模型声明的实际档位展示。运行标题跟随整轮状态，来源使用默认三条的小号灰色编号列表，技能统计按需查看。没有蓝色常驻选中边框，复制没有常驻边框与底色；通用集中外观、通知和更新。

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

输入 `@代码` 可从完整演示目录加入尚未参与的代码专家，并显示以专家 ID 选择的 chip。目录中的“研究”专家特意比已有会话快照新一个版本：输入 `@研究` 可审阅“以最新配置重新引入”的确认与取消，确认后旧上下文进入保留记录，已有消息不改写。新空白会话选中专家只记录选择，发送演示消息时才建立合成成员上下文。

以下验证入口已执行：静态合同通过；Browser Use 在 1440px / 390px 与明暗两主题的 4 组组合全部通过。既有 Inspector、草稿保留与移动导航的 10 组回归也通过。原型截图已导出并实际查看；这不计作真实模型或原生客户端通过：

```sh
node docs/design/prototype/verify_expert_roles.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_expert_roles_browser.cjs
```

第二个脚本只允许本地原型地址，浏览器操作由项目固定的 Browser Use runtime 执行，禁止非原型 origin 请求；覆盖桌面/手机、明暗、声明档位、角色替换确认、ID 提及、重新引入和历史保留，并导出截图。已有 Inspector、移动端、来源展示与页面同步脚本继续保留；这些是合成原型验证，不是真实 Host、真实模型或客户端验收。

专家协作在主会话 `Nola` 回复中以内联过程摘要卡片呈现，并在桌面右侧「上下文 > 参与者」与移动「会话详情」中挂载只读专家会话入口（不在左侧或移动会话列表中混排子会话树）；点击打开只读阅读器。当前子会话可回父会话选中专家继续；重新引入前的旧上下文保留快照，不提供发送或静默升级。`expert-history.js` 仅用内存合成记录。设计约束见 [多智能体导航契约](../shared/MULTI_AGENT_NAVIGATION.md) 与 [专家子会话与历史阅读](../shared/EXPERT_CONVERSATION_HISTORY.md)。
