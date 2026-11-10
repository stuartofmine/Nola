# Nola · 业务原型与交互设计

更新：2026-11-10。新增覆盖当前业务清单的可操作场景原型：**68 个场景、220 个状态、六个平台视角**。既有完整交互原型保留；新入口补齐首次引导、账户授权、模型配置、宿主机、权限、Goal、长任务恢复、MCP/Skill、资料记忆、协作、计划通知及维护流程的正常、异常和恢复状态。

## 打开原型

```sh
python3 -m http.server 8071 --bind 127.0.0.1 --directory docs/design/prototype
```

- [全业务场景原型](scenarios.html)：本地服务地址 `http://127.0.0.1:8071/scenarios.html`。按业务、平台、主题和状态审阅，可直接操作表单、分支、确认与取消；场景可通过 URL 分享到具体状态。
- [完整交互原型](index.html)：本地服务首页。保留实际会话列表、全屏聊天、模型档位、编辑重发、独立检查器、活动、资料和通用设置；设置中的“原型预览”可进入全业务场景。
- [业务覆盖与维护规则](docs/BUSINESS-SCENARIOS.md)：逐项入口、状态数、适用平台和当前 Web 14 路由映射。
- [覆盖 CSV](FEATURE-COVERAGE.csv) / [结构化场景清单](BUSINESS-SCENARIOS.json)：68 项完整清单；旧覆盖表不再把过时的 Today 导航或历史测试当现行结论。
- [验证与边界](docs/VALIDATION.md)：区分本轮合成原型回放、历史记录、原生客户端和真实模型证据。

## 现行设计合同

现行产品行为以[对话交互](../shared/CONVERSATION_UI_DESIGN.md)、[会话检查器](../shared/CONVERSATION_INSPECTOR.md)、[视觉系统](../shared/DESIGN_SYSTEM.md)和[宿主机流程](../shared/HOST_CONNECTION_WORKFLOW.md)为准。原型交互与视觉参数见 [UI-INTERACTION.md](docs/UI-INTERACTION.md)、[DESIGN-SPEC.md](docs/DESIGN-SPEC.md)；[客户端差异](docs/IMPLEMENTATION-COMPARISON.md)继续记录原型与生产之间的距离。

手机仍以对话列表进入聊天，主导航为对话、活动、资料、设置；桌面是单顶栏、内容标签与独立侧栏。检查器区段原地展开，模板归入模型配置，推理按模型声明的实际档位展示。没有蓝色常驻选中边框，复制没有常驻边框与底色；通用集中外观、通知和更新。

## 演示边界

场景预览不调用模型、Host、OAuth、系统权限、文件、调度或 Git，不下载更新、不接受真实凭据。适用平台表示设计覆盖范围，不是生产能力或原生验收声明；CLI 预览是文本布局草图，不增加命令。外部旧 Site 未随本轮更新，不再作为当前资产入口。

新场景原型仅在本页内存保留各平台/场景的草稿与状态，刷新重置。既有完整原型的主题、减少动态效果与专家颜色可使用浏览器存储；其他样例在刷新后重置，没有账号或跨设备同步。旧脚本 `verify.py`、`verify_commands.py` 属于历史记录，不作为本轮通过依据。

## 检查入口

```sh
python3 docs/design/prototype/sync_business_scenarios.py --check
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_business_scenarios.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_business_layout.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/prototype/verify_page_sync.cjs
```

浏览器回放需要本机 Chrome 与已有 Playwright runtime 依赖；缺少依赖时运行 `npm ci --prefix web/playwright_runtime`。

生成清单使用 `sync_business_scenarios.py`，会检查状态分支、源码文件及生产 Web 路由新增遗漏。浏览器脚本阻止外部请求，回放、截图图册和回执默认写入本机 `/tmp`，不进入源码仓库。原型测试通过不代替真实客户端验收。
