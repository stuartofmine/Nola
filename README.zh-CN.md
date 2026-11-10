# Nola

本地优先的个人 AI 助理，包含 macOS、iOS、Android、Windows、Web 与终端客户端。

[English](README.md) · [公开源码](https://github.com/stuartofmine/Nola) · [下载](https://github.com/stuartofmine/Nola/releases) · [文档](docs/README.md)

## 功能

Nola 将对话、模型连接、本地工作区、工具执行、记忆和长任务放在同一个应用中。各端使用共享 Rust 运行时，分别负责界面、系统权限和生命周期。模型请求使用用户配置的连接；各端支持的本地模型运行时独立管理。

CLI 可以通过带认证的本地 HTTP 服务，为 Web 和远程客户端提供 Host 能力。

## 下载

1.0.0 开发预览提供 **macOS Apple Silicon 完整 CLI 包**。请保留 CLI、Agent Host、服务、Web 资源和浏览器运行时的完整目录结构。此包尚未进行 Apple Developer ID 公证。其他平台暂未发布安装包，可从源码构建。

只下载[发行页](https://github.com/stuartofmine/Nola/releases)列出的制品。解压后在目录中运行：

```sh
./nola --version
./nola --help
./nola web --host 127.0.0.1 --port 5001
```

开始对话前，先配置模型连接并选择已声明的模型。凭据只保存在本地配置或平台凭据存储中，不应提交到源码仓库。

## 架构

| 组件 | 实现 | 职责 |
| --- | --- | --- |
| macOS · `NolaMac/` | SwiftUI / AppKit | 原生桌面客户端与平台 Host 集成 |
| iOS · `Nola/` | SwiftUI | 移动客户端与 iOS 系统能力 |
| Android · `android/` | Kotlin / Compose | 移动客户端、JNI 集成与 Android 系统能力 |
| Windows · `windows/` | 桌面原生壳 + Web UI | 桌面客户端与受控本地 Host 集成 |
| Web · `web/frontend/` | Svelte / TypeScript | 由 Rust Host 提供服务的浏览器界面 |
| CLI · `cli/` | Rust | 终端客户端、Agent Host 与本地 HTTP 服务 |
| Core · `core/` | Rust、C ABI / JNI | 领域契约、Agent 执行、Application Service、持久化与 Host 接口 |
| Apple 共享代码 · `Shared/` | Swift | 共用展示组件与协议代码 |

各原生客户端拥有自己的界面；macOS App 不是 CLI 的外壳。对话和持久运行状态由本地 Application Service 与 Store 管理。远程 Host 访问需要认证和明确授权。

## 从源码运行

需要稳定版 Rust 和 Node.js 24 或更新版本。Apple 客户端需要 Xcode；Android 还需要 JDK 21、Android SDK/NDK、Rust Android targets 和 `cargo-ndk`；Windows 需要对应的原生工具链。

在仓库根目录执行：

```sh
npm --prefix web/frontend ci
npm --prefix web/frontend run build
cargo build --locked -p nola_cli -p nola_service --bins
./target/debug/nola --help
./target/debug/nola web --host 127.0.0.1 --port 5001 --web-root "$PWD/web"
```

也可运行 `./dev.sh` 执行上述本地构建与启动，使用 `WEB_HOST`、`WEB_PORT` 设置监听地址。

模型可在 Web/原生客户端设置中配置，也可使用 `nola model apply-config /absolute/path/to/config.json`。`llm.default_model` 必须引用同一份配置中声明的连接和模型。

CLI 交互中，按 `Ctrl+V` 添加剪贴板图片，也可粘贴图片的绝对路径。图片分析需要模型声明图片输入能力。权限拒绝后，用 `/allow` 查看会话权限；配置中的显式拒绝规则和操作系统权限需要在各自来源处修改，然后明确要求重试。

## 测试

按修改范围运行相应检查：

```sh
cargo test --locked -p nola_domain -p nola_protocol -p nola_store -p nola_update_host
npm --prefix web/frontend run check
npm --prefix web/frontend test
./scripts/test_mac_layers.sh contracts
./scripts/run_client_tests.sh
(cd android && ./gradlew test)
```

Apple 测试脚本使用隔离的 Test 输出目录，macOS Debug 构建保留配置的签名身份。Android 设备测试需要已启动的模拟器或明确选定的设备。契约测试不等于真实模型或各平台安装验收；结果和已知限制见验收记录（本地工作记录，不随仓库分发）。

## 设计与验收

[可操作原型](docs/design/prototype/README.md) · [68 项业务／220 个设计状态](docs/design/prototype/docs/BUSINESS-SCENARIOS.md) · [宿主机与原型验收总账](docs/quality/VALIDATION_SCOPE.md)

原型回放、原生界面和真实模型执行分别记录覆盖范围。原型中有某个平台不表示该平台全部业务已经实现或验收。

## 版本与兼容

**本次重新发布的 1.0.0 是首个受支持基线**。Nola 自有协议、ABI、数据库格式与 epoch 均从 **1** 开始，不保留此前开发格式的兼容实现。应用版本/build 与契约版本分别管理；外部标准协议沿用其自身的版本标识。

从本次发布起，后续版本必须维护已发布接口和用户数据的兼容性。协议变更需要明确协商，数据变更需要可验证、可回滚的迁移。未知格式明确拒绝，不自动删除或重建用户数据。详见[版本兼容策略](docs/architecture/总体设计/VERSION_COMPATIBILITY.md)。

## 更新与发布

源码与签名更新信息托管在 GitHub，完整包通过 GitHub Releases 分发。客户端检查 Ed25519 签名、平台、数据格式、文件长度和 SHA-256。检查更新和安装交接均由用户明确操作。GitHub Actions 每周续签更新信息。

维护者操作见[发布说明](release/README.md)。不要提交凭据、签名材料、数据库、日志或生成的构建产物。

## 参与贡献

[贡献指南](CONTRIBUTING.md) · [安全策略](SECURITY.md)

项目采用 [Apache-2.0](LICENSE) 许可证。
