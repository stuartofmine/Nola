> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# Nola

[公开源码](https://github.com/stuartofmine/Nola) · [发行包](https://github.com/stuartofmine/Nola/releases)

当前为开发预览。首次二进制预览提供 macOS Apple Silicon CLI 完整包；其他原生客户端可从源码构建，不能将未发布的平台包视为可用。更新信息有签名校验，安装不会迁移或重建用户数据。参见[发布说明](release/README.md)。

Nola 是一个 **local-first（本地优先）** 的个人智能助理系统。

> English version: [`README.md`](README.md)

## 产品方向

Nola 围绕三个核心方向设计：

- **数据源驱动**：对话可显式使用文档、网页、系统数据、健康摘要和本地工作区作为上下文。
- **自优化**：上下文规划、模型路由、检索和执行策略自动适配，不再向用户暴露过时的固定步数配置。
- **长期个人智能**：稳定偏好与长期记忆会被自动总结、去重，并支持本地优先的查看和管理。

Chat 是产品的主界面。Agent 工具、数据源、记忆和自动化都服务于对话，而不是彼此割裂的独立模块。

## 架构

| 组件 | 技术栈 | 说明 |
|---|---|---|
| iOS (`Nola/`) | SwiftUI | 移动端聊天、HealthKit/系统数据、通知、记忆与本地模型 |
| macOS (`NolaMac/`) | SwiftUI/AppKit | 原生桌面聊天、本机/宿主机切换、工作区、工具与已下载模型 |
| Android (`android/`) | Kotlin/Compose + Hilt | 移动端聊天、Health Connect、平台权限与共享 Agent Runtime |
| 共享核心 (`core/`) | Rust + C ABI/JNI | 确定性的 Agent 状态、权限、工具协议与上下文规划 |
| Web (`web/`) + CLI (`cli/`) | Rust CLI 后端 + Jinja 模板/静态资源 | 独立的本地控制台、自动化宿主与开发者 CLI |
| Backend (`backend/`) | Cloudflare Worker + D1 | OAuth 中继、订阅与用量、模型中转、通知 |

macOS App 不是 CLI 的外壳。普通原生聊天不会启动 CLI 进程；CLI/Web 作为终端工作流和宿主机自动化入口独立存在。

## 仓库结构

```text
.
├── backend/                # 后端服务
├── android/                # Android Compose 客户端
├── cli/                    # Rust CLI 与 Rust Web 后端
├── core/      # 共享 Rust Agent Core 与原生构建脚本
├── Nola/                   # iOS 客户端
├── NolaMac/                # 原生 macOS 客户端
├── NolaTests/              # iOS 单元测试
├── NolaUITests/            # iOS UI 测试
├── NolaMacTests/           # macOS 单元、契约与视觉测试
├── web/                    # Web 模板、静态资源与原生宿主辅助程序
├── Doc/                    # 文档
├── dev.sh                  # 开发启动脚本（默认热重载）
└── deploy.sh               # 发布脚本
```

## 本地开发

### 环境要求

- Node.js + npm
- Rust 工具链（`rustup` + `cargo`）
- iOS/macOS 开发需要 macOS 与 Xcode
- Android 原生构建需要 JDK 21、Android SDK、Android NDK 与 `cargo-ndk`
- 后端、Web 与 CLI 开发需要 macOS/Linux Shell 环境

### 启动开发环境

```bash
./dev.sh
```

默认行为：
- 后端使用 Worker `dev` 模式并自动热重载
- Web 默认由 Rust CLI 后端提供服务

常用开关：

```bash
NOLA_LAN=0 ./dev.sh
BACKEND_PORT=8000 WEB_PORT=5001 ./dev.sh
```

### CLI 交互

CLI 交互中，复制图片后按 `Ctrl+V` 添加图片附件（macOS 终端同样使用此快捷键），也可单独粘贴图片的绝对路径。发送前可删除图片占位符以移除附件。图片能否被模型分析取决于所选模型声明的图片输入能力。

权限拒绝后，用 `/allow` 查看本会话的权限设置；配置中的显式 `deny` 规则需修改对应配置，系统权限需在操作系统中授予。调整后明确要求重试。底部缓存命中率表示最近一次模型请求，`—` 表示服务端未上报完整数据。

### 原生端构建与测试

iOS 测试脚本会自动选择可用模拟器、限制并行 worker 数，并且只清理由本次运行创建的 XCTest 克隆。

```bash
./scripts/run_client_tests.sh
# 可选：PARALLEL_TEST_WORKERS=8 ./scripts/run_client_tests.sh
xcodebuild test -project Nola.xcodeproj -scheme NolaMac \
  -destination 'platform=macOS'
```

Android 会为所有支持的 ABI 构建 Rust 静态库。构建脚本会自动查找 `cargo` 和 NDK，不依赖某台机器的绝对路径。

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi \
  i686-linux-android x86_64-linux-android
cargo install cargo-ndk --locked
cd android
./gradlew test
./gradlew assembleDebugAndroidTest
./gradlew connectedDebugAndroidTest   # 需要已启动的模拟器或真机
```

共享核心与后端检查：

```bash
cd core && cargo test --quiet
cd backend && npm run typecheck
cd backend && python -m pytest -q
```

## 当前平台行为

- **附件**：iOS 与 macOS 在本地提取支持的文档文本。DOCX 支持正文、表格、页眉、页脚、脚注和尾注。附件内容只作为参考上下文，不会自行触发用户命令。
- **健康数据**：iOS 直接读取 HealthKit，Data Tab 与详情页共享同一份健康快照，并响应 HealthKit observer 的变化。macOS 只有在收到 iPhone 同步摘要后才显示健康数据。
- **Agent Runtime**：iOS 与 Android 使用共享 Rust command/event 协议；操作系统授权、沙箱文件、UI 和生命周期仍由各平台负责。
- **生成产物**：XCFramework、静态库、JNI 动态库、日志和本地数据库均为构建产物，不应提交到仓库。

## Release 发布

`deploy.sh` 默认会先校验环境，再执行 `wrangler deploy` 发布 release 版本。

```bash
export nola_backend_env='{"SERVER_JWT_SECRET":"..."}'  # 需完整 JSON
export CLOUDFLARE_API_TOKEN=xxx
./deploy.sh
```

可选参数：

```bash
DEPLOY_RUN_TESTS=1 ./deploy.sh
DEPLOY_RUN_TYPECHECK=0 ./deploy.sh
./deploy.sh --env production
```

## 文档

- 文档总入口：[软件架构、UI、当前任务与历史任务](Doc/README.md)
- 接口文档（中文）：[`Doc/软件架构设计/模块/01-领域与协议/API.zh-CN.md`](Doc/软件架构设计/模块/01-领域与协议/API.zh-CN.md)
- API docs (EN): [`Doc/软件架构设计/模块/01-领域与协议/API.md`](Doc/软件架构设计/模块/01-领域与协议/API.md)
- 需求说明：[`Doc/软件架构设计/总体设计/REQUIREMENTS.md`](Doc/软件架构设计/总体设计/REQUIREMENTS.md)
- 软件设计：[`Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md`](Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md)
- Core / Host 分层架构：[`Doc/软件架构设计/总体设计/CORE_HOST_ARCHITECTURE.md`](Doc/软件架构设计/总体设计/CORE_HOST_ARCHITECTURE.md)
- 移动端共享 Agent Core：[`Doc/软件架构设计/模块/13-客户端投影/MOBILE_SHARED_CORE.md`](Doc/软件架构设计/模块/13-客户端投影/MOBILE_SHARED_CORE.md)
- 变更记录：[`Doc/历史任务/实施与审计/CHANGELOG.md`](Doc/历史任务/实施与审计/CHANGELOG.md)

## 社区协作

- 贡献指南：[`CONTRIBUTING.md`](CONTRIBUTING.md)
- 行为准则：[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md)
- 安全策略：[`SECURITY.md`](SECURITY.md)

## 许可证

本项目采用 **Apache License 2.0** 许可证。  
完整条款见 [`LICENSE`](LICENSE)。

## 配置说明

- 后端仅从 `nola_backend_env`（或 `NOLA_BACKEND_ENV`）JSON 读取配置
- Web 本地配置示例见 `web/.env.example`
- 不要提交生产密钥
