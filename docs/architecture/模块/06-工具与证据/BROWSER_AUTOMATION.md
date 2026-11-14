# 浏览器执行与观察

2026-11-11。受支持的 Nola 自有协议基线仍为 1；本次替换执行实现，不改变已有浏览器工具参数、RPC 路径、权限身份或数据库格式。外部 Browser Use 固定为 `1.0.2`，browser-harness 固定为 `0.1.13`，不随上游 main 自动升级。

## 职责

| 层 | 所有者与行为 |
| --- | --- |
| 模型与权限 | Core 选择工具，Host 核对具体调用、资源、会话和批准后的页面身份；不按自然语言关键词选择浏览器 |
| 会话与观察 | `apps/web/playwright_runtime/bridge.cjs` 保留 owner 队列、页面选择、真实 DOM 句柄、版本、截图、预览与动作/观察分离；Playwright 负责浏览器启动及观察 |
| 浏览器输入 | `browser_use.cjs` 启动每会话独立的 Python worker；`browser_use_runtime/worker.py` 通过 Browser Use 的显式本地 CDP 连接和固定操作驱动导航、点击、键盘、滚轮与文本输入 |
| 用户已有 Chrome | `target=attached` 继续使用用户明确连接的原生浏览器桥；本次不自动接管或关闭用户的浏览器 |
| 移动及远程客户端 | 使用 Host 的同一执行链路及投影，不在手机里安装 Python，也不建立另一套模型循环 |

这里使用 Browser Use 的本地浏览器执行能力，不启动其独立 Agent、云浏览器或模型计费路径。Nola 的任务状态、模型、权限和取消继续由现有链路管理。没有向模型暴露 `browser_exec`、任意 Python 或任意 JavaScript 接口。

## 身份与失败

每个 worker 只连接父进程提供的 `http://127.0.0.1:<port>` 调试端点。每次操作选择父进程已经核对的确切 target；元素通过保留的真实 DOM 句柄一次性绑定，执行前检查仍属于该文档，禁止按相似文本或索引重新定位。首次启动时只关闭 Browser Use 自己新建的空白启动页，不把它计入用户页面，也不关闭原有页面。

Browser Use 的 CDP 调用显式携带当前页面 session。过期 session 明确失败，不能使用库的默认页恢复去重放原输入。输入异常、连接断开或超时都不自动重试；超时结束 worker，防止已放弃的调用在稍后继续执行。动作完成后的观察失败保留动作回执，用户或模型可再次观察，不能因为截图失败重复点击。

Python worker 的私有握手版本为 1，未知版本或不匹配的依赖版本拒绝启动。`NOLA_PLAYWRIGHT_*` 环境变量及原 RPC 保留作为已发布接口兼容名称。Chrome 用户数据目录和当前页面选择格式沿用已有实现，不做删除、重建或隐式格式修复。

worker 不继承模型密钥；关闭 Browser Use 遥测与项目 `.env` 自动加载。Browser Use 的本地运行目录独立于会话文件目录，进程退出时关闭其 daemon。普通操作不访问 Browser Use Cloud，也不需要其 API key。

## 构建与验证

Mac 的浏览器资源构建阶段调用 `scripts/bundle_browser_use_runtime.sh`，使用 uv 安装固定的 Python `3.12.12`，按 `requirements.lock` 安装依赖，并把可重定位解释器、库、许可证和 worker 一起打入应用。构建机需要 uv；已打包应用不要求用户安装 Python、uv 或手工 pip install，运行浏览器动作时也不临时下载依赖。CLI 打包检查同一资源布局，缺 Browser Use 文件时拒绝生成不完整发布包。

源代码开发可先建立隔离环境：

```sh
uv venv .artifacts/browser-use-dev
uv pip install --python .artifacts/browser-use-dev/bin/python -r apps/web/browser_use_runtime/requirements.lock
NOLA_BROWSER_USE_PYTHON="$PWD/.artifacts/browser-use-dev/bin/python" npm run test:browser-use --prefix apps/web/playwright_runtime
python3 -m unittest discover -s apps/web/browser_use_runtime -p 'test_*.py'
npm test --prefix apps/web/playwright_runtime
```

`NOLA_BROWSER_USE_ROOT` 可指定通过构建脚本生成的独立运行时目录，用于验证整体搬移后的包；`NOLA_BROWSER_USE_PYTHON` 仅用于开发时选择已安装固定依赖的解释器。二者是 Host 配置，不接受模型工具参数覆盖。

测试区分工具/RPC 浏览器回归、实际 Chrome 操作、Mac 资源与 broker 接线、发布包布局和真实模型验收。本轮本地 Chrome 操作不调用模型，不能据此宣称真实模型或所有平台 UI 已验收。具体证据保存在本机忽略目录中。

上游参考：[Browser Use](https://github.com/browser-use/browser-use)、[Browser Harness](https://github.com/browser-use/browser-harness)。

## 本轮验证

| 层 | 结果与边界 |
| --- | --- |
| Python 固定操作契约 | 6 项通过：端点、未知操作、过期 session、不重放、失效元素与部分点击失败 |
| Node/原生桥契约 | 28 项通过：握手版本、传输身份、worker 退出/强制终止，以及原有 owner 队列和浏览器身份检查 |
| Rust Host | 35 项浏览器相关测试通过，含实际启动运行时的 Rust 适配端到端；最终固定操作变更后再次通过该端到端用例 |
| Mac | Test 配置 10 项通过，无跳过；正常 Debug 签名构建及深度签名检查通过 |
| CLI 发布布局 | 完整包资源检查通过，固定解释器与 Browser Use 文件随包分发 |
| 实际本地 Chrome | 原有导航/输入/点击/弹窗/截图/重连回归通过；新增两 owner 并行、跨 owner 拒绝、预览 generation、中文/追加/清空/数字/可编辑内容、引用键盘、嵌套滚动、20 轮连续操作与独立停止通过 |

运行时整体搬移测试从已签名应用复制完整资源到另一个目录，使用复制的 Node、Python、库和 worker 重新执行 Chrome 回归；不借用开发虚拟环境。原始日志、回执和运行时副本在本机 `.artifacts/browser-use/` 中保留，不提交到 Git。以上是执行/RPC/包与本地 Chrome 证据，没有调用真实模型，也不代表所有客户端原生 UI 或发布安装验收。
